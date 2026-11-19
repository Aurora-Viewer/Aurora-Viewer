//! What a Director keyframe carries beside its view (`camera::track`): an
//! animation of our own avatar it starts or stops, and its own lens and time
//! of day, each unset by default (the track's or the live value then).
//!
//! Aurora's own: the Legacy Viewer has no camera path tool. The animation is
//! named by its inventory item, never by its asset: a track's file may be
//! shared, and an animation's asset id is not shown nor written outside its
//! item (AGENTS.md, creators' permissions). The lens and the time of day are
//! values of the playback only, the time of day the photo window's sun over
//! the local environment (`world::eep_photo`): nothing is sent to the region.

use crate::director::{Director, Mode};
use glam::Quat;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// An inventory name holds 63 bytes at most (DB_INV_ITEM_NAME_STR_LEN,
/// indra/llinventory/llinventory.h): kept inline, so that a keyframe stays a
/// plain value the track's history copies.
const NAME_BYTES: usize = 63;

/// The name of an animation's item, as it was when it was chosen: shown when
/// the item is no longer in the inventory.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AnimName {
    len: u8,
    bytes: [u8; NAME_BYTES],
}

impl AnimName {
    /// `name` cut to what an inventory name holds, at a character's end.
    pub fn new(name: &str) -> AnimName {
        let mut end = name.len().min(NAME_BYTES);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0; NAME_BYTES];
        bytes[..end].copy_from_slice(&name.as_bytes()[..end]);
        AnimName { len: end as u8, bytes }
    }

    pub fn as_str(&self) -> &str {
        // (built from a whole `str` cut at a character's end)
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }
}

impl Default for AnimName {
    fn default() -> Self {
        AnimName::new("")
    }
}

impl std::fmt::Debug for AnimName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AnimName").field(&self.as_str()).finish()
    }
}

/// An animation of the user's inventory: its item (a link's own id when it
/// was chosen through one) and its name then.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimRef {
    pub item: Uuid,
    pub name: AnimName,
}

impl AnimRef {
    pub fn new(item: Uuid, name: &str) -> AnimRef {
        AnimRef {
            item,
            name: AnimName::new(name),
        }
    }
}

/// What a keyframe carries beside its view; `None` is unset everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KeyExtra {
    /// Started on our avatar when the playback passes this keyframe.
    pub start: Option<AnimRef>,
    /// Stopped here, when the track started it earlier.
    pub stop: Option<AnimRef>,
    /// The lens's f-number here; unset: the track's (`Director::dof`).
    pub aperture: Option<f32>,
    /// Metres the lens is focused at here; unset: the track's focus (the
    /// target in sight, else `Director::focus`).
    pub focus: Option<f32>,
    /// The time of day here, hours from midnight (0 to 24); unset: the
    /// environment's own sun.
    pub hour: Option<f32>,
}

/// An animation as a track's file keeps it: its item's id and its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimFile {
    pub item: String,
    #[serde(default)]
    pub name: String,
}

impl AnimFile {
    fn of(anim: &AnimRef) -> AnimFile {
        AnimFile {
            item: anim.item.to_string(),
            name: anim.name.as_str().to_owned(),
        }
    }

    /// A file is data from outside: an id that is none, and no animation.
    fn read(&self) -> Option<AnimRef> {
        Uuid::parse_str(&self.item)
            .ok()
            .filter(|id| !id.is_nil())
            .map(|item| AnimRef::new(item, &self.name))
    }
}

/// A keyframe's `KeyExtra` as a track's file keeps it (`KeyFile::extra`):
/// absent from a keyframe that sets nothing, and from older files.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ExtraFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<AnimFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<AnimFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aperture: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hour: Option<f32>,
}

impl ExtraFile {
    pub fn is_empty(&self) -> bool {
        *self == ExtraFile::default()
    }

    pub fn of(extra: &KeyExtra) -> ExtraFile {
        ExtraFile {
            start: extra.start.as_ref().map(AnimFile::of),
            stop: extra.stop.as_ref().map(AnimFile::of),
            aperture: extra.aperture,
            focus: extra.focus,
            hour: extra.hour,
        }
    }

    /// Values out of range brought in, those that are no number dropped.
    pub fn read(&self) -> KeyExtra {
        let within = |value: Option<f32>, range: std::ops::RangeInclusive<f32>| {
            value.filter(|v| v.is_finite()).map(|v| v.clamp(*range.start(), *range.end()))
        };
        KeyExtra {
            start: self.start.as_ref().and_then(AnimFile::read),
            stop: self.stop.as_ref().and_then(AnimFile::read),
            aperture: within(self.aperture, crate::director::APERTURE_RANGE),
            focus: within(self.focus, crate::director::FOCUS_RANGE),
            hour: self.hour.filter(|h| h.is_finite()).map(|h| h.rem_euclid(24.0)),
        }
    }
}

/// The time of day a keyframe is given at first: noon.
pub const DEFAULT_HOUR: f32 = 12.0;

/// The sun's rotation at `hour` (0 to 24): risen in the east at six, at the
/// top of the sky at noon, set in the west at six in the evening, under the
/// ground at midnight. As the photo window's sun (`rotation_from`), the
/// sun's direction is this rotation applied to +X.
pub fn sun_at(hour: f32) -> Quat {
    let angle = (hour - 6.0) / 24.0 * std::f32::consts::TAU;
    Quat::from_rotation_y(-angle)
}

/// From `a` to `b` hours by the short way round the clock, `s` of the way.
fn hour_between(a: f32, b: f32, s: f32) -> f32 {
    let mut delta = (b - a).rem_euclid(24.0);
    if delta > 12.0 {
        delta -= 24.0;
    }
    (a + delta * s).rem_euclid(24.0)
}

/// The track gives the view (it plays, or its playhead is held).
fn on_track(d: &Director) -> bool {
    matches!(d.mode, Mode::Scrubbing | Mode::Playing { .. })
}

/// The sun the track gives now: each keyframe's own time of day, or `live`
/// (the environment's sun) where one is unset, the sun going smoothly from
/// one keyframe's to the next's. `None`: the environment's own sun (the
/// track is not played, or no keyframe around the playhead sets a time).
pub fn sun(d: &Director, live: Option<Quat>) -> Option<Quat> {
    if !on_track(d) {
        return None;
    }
    let (before, after, s) = d.track.around(d.time)?;
    let keys = d.track.keys();
    match (keys[before].extra.hour, keys[after].extra.hour) {
        (None, None) => None,
        (Some(a), Some(b)) => Some(sun_at(hour_between(a, b, s))),
        (a, b) => {
            // the live sun on one side: a turn between the two rotations
            let live = live?;
            let rotation = |hour: Option<f32>| hour.map_or(live, sun_at);
            Some(rotation(a).slerp(rotation(b), s).normalize())
        }
    }
}

/// The lens the track gives now: the f-number (`None`: no depth of field of
/// the Director's) and the focus distance, from the track's own `aperture`
/// and `distance` and each keyframe's values. Between two keyframes the
/// focus goes smoothly from one's to the next's; the f-number too, unless
/// one side has no depth of field (it changes half way then). While the
/// track does not give the view, the track's values as they are.
pub fn lens(d: &Director, aperture: Option<f32>, distance: f32) -> (Option<f32>, f32) {
    let Some((before, after, s)) = on_track(d).then(|| d.track.around(d.time)).flatten() else {
        return (aperture, distance);
    };
    let keys = d.track.keys();
    let (a, b) = (&keys[before].extra, &keys[after].extra);
    let f_number = match (a.aperture.or(aperture), b.aperture.or(aperture)) {
        // (the same both sides: exactly it, not its logarithm's round trip)
        (Some(x), Some(y)) if x == y => Some(x),
        // (in stops: the blur changes evenly)
        (Some(x), Some(y)) => Some((x.ln() + (y.ln() - x.ln()) * s).exp()),
        (x, y) => {
            if s < 0.5 {
                x
            } else {
                y
            }
        }
    };
    let (x, y) = (a.focus.unwrap_or(distance), b.focus.unwrap_or(distance));
    (f_number, x + (y - x) * s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::{Pose, Track, TrackFile};
    use glam::Vec3;

    fn three() -> Director {
        let mut d = Director::default();
        for x in [0.0, 10.0, 20.0] {
            d.add_key(Pose::from_view(Vec3::new(x, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0));
        }
        d
    }

    fn set(d: &mut Director, index: usize, edit: impl FnOnce(&mut KeyExtra)) {
        let mut extra = d.track.keys()[index].extra;
        edit(&mut extra);
        d.track.set_extra(index, extra);
    }

    #[test]
    fn a_name_is_kept_as_an_inventory_name_holds_it() {
        assert_eq!(AnimName::new("Dance 1").as_str(), "Dance 1");
        assert_eq!(AnimName::default().as_str(), "");
        let long = "é".repeat(40);
        let kept = AnimName::new(&long);
        // cut at a character's end, never inside one
        assert_eq!(kept.as_str().len(), 62);
        assert!(long.starts_with(kept.as_str()));
        assert_eq!(format!("{:?}", AnimName::new("x")), "AnimName(\"x\")");
    }

    #[test]
    fn the_sun_rises_in_the_east_and_is_up_at_noon() {
        let dir = |hour| sun_at(hour) * Vec3::X;
        assert!(dir(6.0).distance(Vec3::X) < 1e-4);
        assert!(dir(12.0).distance(Vec3::Z) < 1e-4);
        assert!(dir(18.0).distance(-Vec3::X) < 1e-4);
        assert!(dir(0.0).distance(-Vec3::Z) < 1e-4);
        // by the short way round the clock
        assert!((hour_between(23.0, 1.0, 0.5) - 0.0).abs() < 1e-4);
        assert!((hour_between(1.0, 23.0, 0.25) - 0.5).abs() < 1e-4);
        assert!((hour_between(6.0, 18.0, 0.5) - 12.0).abs() < 1e-4);
    }

    #[test]
    fn the_time_of_day_goes_from_a_keyframe_to_the_next() {
        let mut d = three();
        let live = Some(sun_at(9.0));
        // nothing set, or not playing: the environment's own sun
        d.scrub(1.5);
        assert_eq!(sun(&d, live), None);
        set(&mut d, 0, |e| e.hour = Some(18.0));
        set(&mut d, 1, |e| e.hour = Some(22.0));
        d.stop();
        assert_eq!(sun(&d, live), None, "editing: the user's own light");
        d.scrub(0.0);
        assert!(sun(&d, live).expect("sun").angle_between(sun_at(18.0)) < 1e-4);
        d.scrub(1.5);
        assert!(sun(&d, live).expect("sun").angle_between(sun_at(20.0)) < 1e-3);
        // an unset keyframe after: towards the live sun
        d.scrub(4.5);
        let half = sun(&d, live).expect("sun");
        assert!(half.angle_between(sun_at(22.0)) > 0.1 && half.angle_between(sun_at(9.0)) > 0.1);
        d.scrub(6.0);
        assert!(sun(&d, live).expect("sun").angle_between(sun_at(9.0)) < 1e-4);
        // no live sun known: none rather than a guess
        d.scrub(4.5);
        assert_eq!(sun(&d, None), None);
    }

    #[test]
    fn a_keyframe_lens_is_followed_and_an_unset_one_keeps_the_tracks() {
        let mut d = three();
        // nothing set: the track's lens
        d.scrub(1.5);
        assert_eq!(lens(&d, Some(2.8), 5.0), (Some(2.8), 5.0));
        assert_eq!(lens(&d, None, 5.0), (None, 5.0));
        set(&mut d, 1, |e| {
            e.aperture = Some(11.2);
            e.focus = Some(1.0);
        });
        d.scrub(3.0);
        let (f, focus) = lens(&d, Some(2.8), 5.0);
        assert!((f.expect("f") - 11.2).abs() < 1e-3 && (focus - 1.0).abs() < 1e-4);
        // half way from f/2.8 to f/11.2: two stops each side, f/5.6
        d.scrub(1.5);
        let (f, focus) = lens(&d, Some(2.8), 5.0);
        assert!((f.expect("f") - 5.6).abs() < 1e-3, "{f:?}");
        assert!((focus - 3.0).abs() < 1e-4);
        // the track without depth of field: the keyframe's turns it on, half way
        assert_eq!(lens(&d, None, 5.0).0, Some(11.2));
        d.scrub(0.9);
        assert_eq!(lens(&d, None, 5.0).0, None);
        // editing: the track's own values
        d.stop();
        assert_eq!(lens(&d, Some(2.8), 5.0), (Some(2.8), 5.0));
    }

    #[test]
    fn a_keyframe_extra_comes_back_from_its_file() {
        let mut d = three();
        let dance = AnimRef::new(Uuid::from_u128(9), "Dance");
        set(&mut d, 0, |e| e.start = Some(dance));
        set(&mut d, 2, |e| {
            e.stop = Some(dance);
            e.aperture = Some(4.0);
            e.focus = Some(2.5);
            e.hour = Some(19.5);
        });
        let file = d.track.to_file((0, 0));
        // a keyframe that sets nothing writes nothing more
        let text = serde_json::to_string(&file).expect("json");
        assert_eq!(text.matches("\"extra\"").count(), 2);
        // the item's id, never an asset's
        assert!(text.contains(&Uuid::from_u128(9).to_string()));
        let read: TrackFile = serde_json::from_str(&text).expect("json");
        let neck = d.lock.aim();
        let back = Track::from_file(&read, (0, 0), neck).expect("track");
        for (a, b) in back.keys().iter().zip(d.track.keys()) {
            assert_eq!(a.extra, b.extra);
        }
        // the file's version is the same: an older viewer reads the rest
        assert_eq!(file.version, crate::camera::track::FILE_VERSION);
        // a file is data from outside
        let odd = ExtraFile {
            start: Some(AnimFile {
                item: "not an id".into(),
                name: "x".into(),
            }),
            stop: Some(AnimFile {
                item: Uuid::nil().to_string(),
                name: String::new(),
            }),
            aperture: Some(f32::NAN),
            focus: Some(1e9),
            hour: Some(-1.0),
        }
        .read();
        assert_eq!((odd.start, odd.stop, odd.aperture), (None, None, None));
        assert_eq!(odd.focus, Some(*crate::director::FOCUS_RANGE.end()));
        assert_eq!(odd.hour, Some(23.0));
        // an older file, without them: nothing set
        let old = r#"{"version":2,"keys":[{"time":0.0,"position":[1.0,2.0,3.0],"rotation":[0.0,0.0,0.0,1.0],"fov":1.0}]}"#;
        let old = Track::from_file(&serde_json::from_str(old).expect("json"), (0, 0), neck).expect("track");
        assert_eq!(old.keys()[0].extra, KeyExtra::default());
    }
}
