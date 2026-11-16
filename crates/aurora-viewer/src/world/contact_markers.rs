//! Markers over contact-set members (an Aurora addition, no Legacy Viewer
//! counterpart): which residents get one and where it stands, from one
//! frame to the next. `ui/contact_markers.rs` projects and paints them.
//!
//! Beyond the avatars the region sends us, all we have is a coarse location
//! about once a second, its altitude in 4 m steps and sometimes unknown
//! (`world/radar.rs`). So a marker glides between coarse positions, keeps
//! the last altitude it knew, and eases into the exact position when the
//! avatar loads. Parcels that hide their avatars need nothing here: the
//! simulator does not send them, so this reveals no more than the mini-map.

use crate::world::contact_sets::{Marker, MarkerScope};
use glam::{DVec3, Vec2, Vec3};
use std::collections::HashMap;
use uuid::Uuid;

/// Seconds a marker outlives the last sign its resident was in scope: a
/// resident on a parcel line, or a coarse update that comes late, does not
/// make it blink.
pub const GRACE: f64 = 2.0;
/// Seconds a marker takes to appear or to go.
const PRESENCE_EASE: f32 = 0.2;
/// Time constant of the glide between two coarse positions (seconds).
const COARSE_GLIDE: f32 = 0.45;
/// Time constant of what is left of the coarse position once exact.
const EXACT_BLEND: f32 = 0.15;
/// A coarse jump this long (metres) is a teleport, not a walk.
const COARSE_SNAP: f32 = 64.0;
/// Time constant of the lift above the plate and of the turn along the
/// screen edge (seconds).
const SCREEN_EASE: f32 = 0.1;
/// Metres over which a marker fades, past its fade-out distance.
const FADE_BAND: f32 = 2.0;
/// Camera distance from which a marker lands on its plate: where a plate
/// becomes complete (`ui::nameplate::FULL_DISTANCE`). It leaves it a little
/// farther, so that a resident at that distance does not make it hesitate,
/// but before the plate shrinks: it leaves from the badge it landed on.
pub const MERGE_DISTANCE: f32 = crate::ui::nameplate::FULL_DISTANCE;
const MERGE_MARGIN: f32 = crate::ui::nameplate::LEVEL_MARGIN - 0.5;
/// Seconds a marker takes to land on its plate.
const MERGE_EASE: f32 = 0.3;
/// How far through its landing a marker starts to go: the rest of the way
/// it hands over to the plate, which is the only thing left once landed.
const MERGE_HANDOVER: f32 = 0.7;

/// What a marker keeps of its opacity at `merge` (eased, 0..1): it hands
/// over to the plate as it lands, so nothing of it clips through the world
/// where the plate itself would be hidden.
pub fn landing_alpha(merge: f32) -> f32 {
    ((1.0 - merge) / (1.0 - MERGE_HANDOVER)).clamp(0.0, 1.0)
}

/// A resident seen this frame, in render space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sighting {
    pub id: Uuid,
    pub position: Vec3,
    pub altitude_known: bool,
    /// Loaded avatar, as opposed to a coarse location.
    pub exact: bool,
    /// None while our parcel is not known.
    pub in_scope: Option<bool>,
    pub marker: Marker,
}

/// Is a resident at `global` in `scope`? `parcel` is the bitmap and the
/// region corner of our parcel, None while it is unknown (arrival, parcel
/// crossing): that gives None rather than a marker lost for nothing.
pub fn in_scope(scope: MarkerScope, same_region: bool, parcel: Option<(&[u8], (u32, u32))>, global: DVec3) -> Option<bool> {
    if !same_region {
        return Some(false);
    }
    match scope {
        MarkerScope::Off => Some(false),
        MarkerScope::Region => Some(true),
        MarkerScope::Parcel => parcel.map(|(bitmap, origin)| super::landmarks::in_parcel(bitmap, origin, global)),
    }
}

/// How much of a marker shows at `distance` from our avatar: nothing up to
/// the fade-out distance, everything `FADE_BAND` past it.
pub fn proximity_alpha(distance: f32, fade: Option<u16>) -> f32 {
    let Some(fade) = fade else { return 1.0 };
    let t = ((distance - f32::from(fade)) / FADE_BAND).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `a` turned toward `b` by the eased share of the angle between them, the
/// short way: a resident straight behind the camera sends `b` around the
/// circle, and the marker must not jump from one edge to the other.
pub fn ease_direction(a: Vec2, b: Vec2, dt: f32) -> Vec2 {
    let angle = a.angle_to(b);
    Vec2::from_angle(angle * ease(dt, SCREEN_EASE)).rotate(a).normalize_or(b)
}

fn ease(dt: f32, time_constant: f32) -> f32 {
    1.0 - (-dt / time_constant).exp()
}

/// A marker from one frame to the next.
#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub position: Vec3,
    pub marker: Marker,
    /// 0..1, appearing or going.
    pub presence: f32,
    /// Points above the avatar, set by the painter (`ease_lift`); None until
    /// it first stands on the screen.
    lift: Option<f32>,
    /// From the screen centre, set by the painter.
    pub direction: Option<Vec2>,
    /// 0 above the plate, 1 landed on it, linear.
    merge: f32,
    /// Where it lands, from the bottom centre of the plate, and how wide,
    /// set by the painter while landing. Kept as it leaves: the plate then
    /// changes size and offers another place, on the far side.
    pub badge: Option<(Vec2, f32)>,
    altitude: Option<f32>,
    exact: bool,
    /// What remains between the coarse position and the exact one.
    blend: Vec3,
    in_scope_at: f64,
}

impl Entry {
    /// Whether a plate offers the marker a place and the camera is near
    /// enough for it to go there.
    pub fn landing(&self, slot: bool, camera_distance: f32) -> bool {
        let reach = if self.merge > 0.0 {
            MERGE_DISTANCE + MERGE_MARGIN
        } else {
            MERGE_DISTANCE
        };
        slot && camera_distance <= reach
    }

    /// The marker moves toward its plate while `landing`; returns how far
    /// it is, eased.
    pub fn ease_merge(&mut self, slot: bool, camera_distance: f32, dt: f32) -> f32 {
        let step = dt / MERGE_EASE;
        let landing = self.landing(slot, camera_distance);
        self.merge = (self.merge + if landing { step } else { -step }).clamp(0.0, 1.0);
        self.merge * self.merge * (3.0 - 2.0 * self.merge)
    }

    /// No plate left to land on: back above the avatar at once.
    pub fn drop_merge(&mut self) {
        self.merge = 0.0;
        self.badge = None;
    }

    /// The lift eased toward `target` over `dt` seconds, and returned. A
    /// marker's first lift is `target` itself: it does not slide up from the
    /// avatar's root on the frame it appears.
    pub fn ease_lift(&mut self, target: f32, dt: f32) -> f32 {
        let lift = self.lift.map_or(target, |lift| lift + (target - lift) * ease(dt, SCREEN_EASE));
        self.lift = Some(lift);
        lift
    }
}

#[derive(Debug, Default)]
pub struct Tracker {
    entries: HashMap<Uuid, Entry>,
    region: Option<u64>,
    last: Option<f64>,
}

impl Tracker {
    pub fn clear(&mut self) {
        self.entries.clear();
        self.last = None;
    }

    /// One frame: `marked` says whether a resident may still have a marker
    /// at all (set disabled, resident blocked: gone at once). `fallback_z`
    /// stands for an altitude nobody knows yet: ours. Returns the seconds
    /// since the last frame.
    pub fn update(&mut self, region: u64, now: f64, sightings: &[Sighting], fallback_z: f32, marked: impl Fn(&Uuid) -> bool) -> f32 {
        // positions are relative to the region we are in
        if self.region != Some(region) {
            self.clear();
            self.region = Some(region);
        }
        let dt = self.last.map_or(0.0, |last| (now - last).clamp(0.0, 0.25)) as f32;
        self.last = Some(now);
        for s in sightings {
            let entry = match (self.entries.get_mut(&s.id), s.in_scope) {
                (Some(entry), _) => entry,
                (None, Some(true)) => self.entries.entry(s.id).or_insert(Entry {
                    position: s.position,
                    marker: s.marker,
                    presence: 0.0,
                    lift: None,
                    direction: None,
                    merge: 0.0,
                    badge: None,
                    altitude: None,
                    exact: s.exact,
                    blend: Vec3::ZERO,
                    in_scope_at: now,
                }),
                _ => continue,
            };
            // an unknown parcel keeps what we have, like a known yes
            if s.in_scope != Some(false) {
                entry.in_scope_at = now;
            }
            entry.marker = s.marker;
            if s.altitude_known {
                entry.altitude = Some(s.position.z);
            }
            let z = entry.altitude.unwrap_or(fallback_z);
            let target = s.position.truncate().extend(z);
            if s.exact {
                if !entry.exact {
                    entry.blend = entry.position - target;
                }
                entry.blend *= 1.0 - ease(dt, EXACT_BLEND);
                entry.position = target + entry.blend;
            } else if entry.position.distance(target) > COARSE_SNAP {
                entry.position = target;
            } else {
                entry.position = entry.position.lerp(target, ease(dt, COARSE_GLIDE));
            }
            entry.exact = s.exact;
        }
        self.entries.retain(|id, entry| {
            let wanted = now - entry.in_scope_at <= GRACE;
            let step = dt / PRESENCE_EASE;
            entry.presence = (entry.presence + if wanted { step } else { -step }).clamp(0.0, 1.0);
            marked(id) && (wanted || entry.presence > 0.0)
        });
        dt
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&Uuid, &mut Entry)> {
        self.entries.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: Uuid = Uuid::from_u128(1);
    const MARKER: Marker = Marker {
        scope: MarkerScope::Region,
        shape: crate::world::contact_sets::MarkerShape::Circle,
        color: [1.0, 0.0, 0.0, 1.0],
        border: None,
        background: None,
        icon: None,
        fade: None,
    };

    fn seen(position: Vec3, exact: bool, in_scope: Option<bool>) -> Sighting {
        Sighting {
            id: ID,
            position,
            altitude_known: true,
            exact,
            in_scope,
            marker: MARKER,
        }
    }

    /// Runs `seconds` at 60 frames a second from `start`.
    fn run(tracker: &mut Tracker, start: f64, seconds: f64, list: &[Sighting]) -> f64 {
        let mut now = start;
        for _ in 0..(seconds * 60.0) as usize {
            now += 1.0 / 60.0;
            tracker.update(1, now, list, 20.0, |_| true);
        }
        now
    }

    fn position(tracker: &mut Tracker) -> Vec3 {
        tracker.iter_mut().next().expect("a marker").1.position
    }

    #[test]
    fn scopes_follow_the_region_and_the_parcel_bitmap() {
        let mut bitmap = vec![0u8; 512];
        bitmap[0] = 1; // the first 4 m cell
        let parcel = Some((bitmap.as_slice(), (1000, 2000)));
        let inside = DVec3::new(1002.0, 2002.0, 20.0);
        let outside = DVec3::new(1010.0, 2002.0, 20.0);
        assert_eq!(in_scope(MarkerScope::Parcel, true, parcel, inside), Some(true));
        assert_eq!(in_scope(MarkerScope::Parcel, true, parcel, outside), Some(false));
        assert_eq!(in_scope(MarkerScope::Parcel, false, parcel, inside), Some(false));
        assert_eq!(in_scope(MarkerScope::Parcel, true, None, inside), None, "our parcel is not known");
        assert_eq!(in_scope(MarkerScope::Region, true, None, outside), Some(true));
        assert_eq!(in_scope(MarkerScope::Region, false, None, outside), Some(false));
        assert_eq!(in_scope(MarkerScope::Off, true, parcel, inside), Some(false));
    }

    #[test]
    fn the_fade_starts_at_its_distance_and_ends_past_it() {
        assert_eq!(proximity_alpha(0.0, None), 1.0);
        assert_eq!(proximity_alpha(3.0, Some(10)), 0.0);
        assert_eq!(proximity_alpha(10.0, Some(10)), 0.0);
        assert_eq!(proximity_alpha(11.0, Some(10)), 0.5);
        assert_eq!(proximity_alpha(12.0, Some(10)), 1.0);
    }

    #[test]
    fn the_direction_turns_the_short_way_without_jumping() {
        let eased = ease_direction(Vec2::X, Vec2::Y, 0.01);
        assert!(eased.y > 0.0 && eased.y < 0.2 && eased.x > 0.9);
        let eased = ease_direction(Vec2::X, -Vec2::Y, 0.01);
        assert!(eased.y < 0.0, "a quarter turn the other way");
        let mut d = Vec2::X;
        for _ in 0..120 {
            d = ease_direction(d, Vec2::Y, 1.0 / 60.0);
        }
        assert!(d.distance(Vec2::Y) < 1e-3);
    }

    #[test]
    fn a_coarse_marker_glides_then_settles() {
        let mut t = Tracker::default();
        let list = [seen(Vec3::new(10.0, 10.0, 20.0), false, Some(true))];
        let now = run(&mut t, 0.0, 0.1, &list);
        let list = [seen(Vec3::new(14.0, 10.0, 20.0), false, Some(true))];
        let now = run(&mut t, now, 1.0 / 30.0, &list);
        let x = position(&mut t).x;
        assert!(x > 10.0 && x < 11.0, "starts gliding: {x}");
        run(&mut t, now, 4.0, &list);
        assert!(position(&mut t).distance(Vec3::new(14.0, 10.0, 20.0)) < 0.05);
    }

    #[test]
    fn a_coarse_teleport_snaps_instead_of_crossing_the_region() {
        let mut t = Tracker::default();
        let now = run(&mut t, 0.0, 0.1, &[seen(Vec3::new(10.0, 10.0, 20.0), false, Some(true))]);
        run(&mut t, now, 0.05, &[seen(Vec3::new(200.0, 10.0, 20.0), false, Some(true))]);
        assert_eq!(position(&mut t).x, 200.0);
    }

    #[test]
    fn an_avatar_that_loads_eases_into_its_exact_position() {
        let mut t = Tracker::default();
        let now = run(&mut t, 0.0, 0.1, &[seen(Vec3::new(10.0, 10.0, 20.0), false, Some(true))]);
        let exact = [seen(Vec3::new(13.0, 10.0, 21.0), true, Some(true))];
        let now = run(&mut t, now, 1.0 / 30.0, &exact);
        let p = position(&mut t);
        assert!(p.x > 10.0 && p.x < 12.9, "no jump: {p}");
        run(&mut t, now, 2.0, &exact);
        assert!(position(&mut t).distance(exact[0].position) < 0.01);
    }

    #[test]
    fn an_exact_marker_follows_its_avatar_without_lag() {
        let mut t = Tracker::default();
        let mut now = run(&mut t, 0.0, 0.1, &[seen(Vec3::new(0.0, 0.0, 20.0), true, Some(true))]);
        for frame in 0..60 {
            now += 1.0 / 60.0;
            let p = Vec3::new(frame as f32 * 0.25, 0.0, 20.0);
            t.update(1, now, &[seen(p, true, Some(true))], 20.0, |_| true);
            assert_eq!(position(&mut t), p);
        }
    }

    #[test]
    fn an_unknown_altitude_keeps_the_last_known_one_else_ours() {
        let mut t = Tracker::default();
        let mut unknown = seen(Vec3::new(10.0, 10.0, 1020.0), false, Some(true));
        unknown.altitude_known = false;
        let now = run(&mut t, 0.0, 0.1, &[unknown]);
        assert_eq!(position(&mut t).z, 20.0, "our own altitude");
        let now = run(&mut t, now, 3.0, &[seen(Vec3::new(10.0, 10.0, 300.0), false, Some(true))]);
        assert!(position(&mut t).z > 290.0);
        run(&mut t, now, 3.0, &[unknown]);
        assert!(position(&mut t).z > 295.0, "keeps the altitude it knew");
    }

    #[test]
    fn a_resident_leaving_scope_lasts_the_grace_then_fades() {
        let mut t = Tracker::default();
        let p = Vec3::new(10.0, 10.0, 20.0);
        let now = run(&mut t, 0.0, 1.0, &[seen(p, true, Some(true))]);
        assert_eq!(t.iter_mut().next().expect("shown").1.presence, 1.0);
        let out = [seen(p, true, Some(false))];
        let now = run(&mut t, now, GRACE - 0.1, &out);
        assert_eq!(t.iter_mut().next().expect("grace").1.presence, 1.0);
        run(&mut t, now, 0.5, &out);
        assert_eq!(t.iter_mut().count(), 0);
    }

    #[test]
    fn an_unknown_parcel_keeps_a_marker_and_starts_none() {
        let mut t = Tracker::default();
        let p = Vec3::new(10.0, 10.0, 20.0);
        run(&mut t, 0.0, 1.0, &[seen(p, true, None)]);
        assert_eq!(t.iter_mut().count(), 0, "nothing new without our parcel");
        let now = run(&mut t, 1.0, 1.0, &[seen(p, true, Some(true))]);
        run(&mut t, now, GRACE * 2.0, &[seen(p, true, None)]);
        assert_eq!(t.iter_mut().count(), 1);
    }

    #[test]
    fn a_landed_marker_leaves_only_the_plate() {
        assert_eq!(landing_alpha(0.0), 1.0);
        assert_eq!(landing_alpha(MERGE_HANDOVER), 1.0);
        assert!(landing_alpha(0.85) > 0.0 && landing_alpha(0.85) < 1.0);
        assert_eq!(landing_alpha(1.0), 0.0);
    }

    #[test]
    fn a_marker_lands_on_a_near_plate_and_leaves_it_a_little_farther() {
        let mut t = Tracker::default();
        run(&mut t, 0.0, 0.1, &[seen(Vec3::new(10.0, 10.0, 20.0), true, Some(true))]);
        let (_, entry) = t.iter_mut().next().expect("a marker");
        let mut settle = |slot, distance| {
            let mut merge = 0.0;
            for _ in 0..60 {
                merge = entry.ease_merge(slot, distance, 1.0 / 60.0);
            }
            merge
        };
        assert_eq!(settle(true, MERGE_DISTANCE + 1.0), 0.0, "too far");
        assert_eq!(settle(false, 5.0), 0.0, "no place on the plate");
        assert_eq!(settle(true, MERGE_DISTANCE), 1.0);
        assert_eq!(settle(true, MERGE_DISTANCE + 1.0), 1.0, "stays past the threshold");
        assert_eq!(settle(true, MERGE_DISTANCE + MERGE_MARGIN + 1.0), 0.0);
        assert_eq!(settle(true, 5.0), 1.0);
        assert_eq!(settle(false, 5.0), 0.0, "the plate is gone");
    }

    #[test]
    fn a_new_marker_starts_at_its_lift_then_eases() {
        let mut t = Tracker::default();
        run(&mut t, 0.0, 0.1, &[seen(Vec3::new(10.0, 10.0, 20.0), true, Some(true))]);
        let (_, entry) = t.iter_mut().next().expect("a marker");
        assert_eq!(entry.ease_lift(30.0, 1.0 / 60.0), 30.0, "no slide up from the root");
        let eased = entry.ease_lift(50.0, 1.0 / 60.0);
        assert!(eased > 30.0 && eased < 50.0, "eases afterwards: {eased}");
    }

    #[test]
    fn a_disabled_marker_goes_at_once_and_a_new_region_starts_over() {
        let mut t = Tracker::default();
        let list = [seen(Vec3::new(10.0, 10.0, 20.0), true, Some(true))];
        run(&mut t, 0.0, 1.0, &list);
        t.update(1, 2.0, &[], 20.0, |_| false);
        assert_eq!(t.iter_mut().count(), 0);
        run(&mut t, 2.0, 1.0, &list);
        t.update(2, 4.0, &[], 20.0, |_| true);
        assert_eq!(t.iter_mut().count(), 0);
    }
}
