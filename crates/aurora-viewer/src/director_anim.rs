//! The animations a Director track plays on our own avatar: a keyframe
//! starts one when the playback passes it (`KeyExtra::start`), a later one
//! may stop it (`KeyExtra::stop`); whatever the track started stops when
//! the playback ends, however it ends (its end, Escape, a teleport, the
//! window closed). This is the schedule alone; the app plays each one as
//! the inventory's "Play in world" does (`app/director_anim.rs`).
//!
//! Aurora's own: the Legacy Viewer has no camera path tool.

use crate::camera::track::Track;
use crate::director::{Director, Mode};
use uuid::Uuid;

/// What the schedule asks of the avatar: an animation of the inventory
/// (its item) to start or to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cue {
    Start(Uuid),
    Stop(Uuid),
}

/// The animations the playback started, and where it was last frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Schedule {
    /// The items started and not stopped yet, in the order they started.
    playing: Vec<Uuid>,
    /// The playhead last frame while the track ran; `None` before it ran.
    at: Option<f32>,
}

impl Schedule {
    #[cfg(test)]
    pub fn playing(&self) -> &[Uuid] {
        &self.playing
    }

    /// Each frame, after the playback's clock: what passing the keyframes
    /// since the last frame starts and stops. The track runs once its start
    /// delay is over (the first view is held for the recording to start:
    /// the first keyframe's animation starts with the camera), and through
    /// the hold on its last view. A playback that starts from the middle
    /// plays what the keyframes before the playhead left playing; a loop
    /// stops everything at its end and starts over. Anything but the track
    /// running stops all it started.
    pub fn update(&mut self, d: &Director) -> Vec<Cue> {
        let running = matches!(d.mode, Mode::Playing { wait, .. } if wait <= 0.0);
        if !running || d.track.len() < 2 {
            return self.stop_all();
        }
        let time = d.time;
        let mut cues = Vec::new();
        match self.at {
            None => self.resume(&d.track, time, &mut cues),
            Some(at) if time + 1e-4 < at => {
                // looped: the end passed, then the start again
                self.pass(&d.track, at, d.track.duration(), &mut cues);
                cues.extend(self.stop_all());
                self.resume(&d.track, time, &mut cues);
            }
            Some(at) => self.pass(&d.track, at, time, &mut cues),
        }
        self.at = Some(time);
        cues
    }

    /// Everything the track started stops (the newest first).
    pub fn stop_all(&mut self) -> Vec<Cue> {
        self.at = None;
        self.playing.drain(..).rev().map(Cue::Stop).collect()
    }

    /// What the keyframes up to `time` (included) leave playing, started at
    /// once: a start and its stop both behind the playhead play nothing.
    fn resume(&mut self, track: &Track, time: f32, cues: &mut Vec<Cue>) {
        let mut playing = Vec::new();
        for key in track.keys().iter().filter(|key| key.time <= time) {
            if let Some(stop) = key.extra.stop {
                playing.retain(|item| *item != stop.item);
            }
            if let Some(start) = key.extra.start {
                playing.retain(|item| *item != start.item);
                playing.push(start.item);
            }
        }
        cues.extend(playing.iter().copied().map(Cue::Start));
        self.playing = playing;
        self.at = Some(time);
    }

    /// The keyframes after `from` up to `to` (included), in order: a stop,
    /// then a start (one already playing starts over).
    fn pass(&mut self, track: &Track, from: f32, to: f32, cues: &mut Vec<Cue>) {
        for key in track.keys().iter().filter(|key| key.time > from && key.time <= to) {
            if let Some(stop) = key.extra.stop
                && self.take(stop.item)
            {
                cues.push(Cue::Stop(stop.item));
            }
            if let Some(start) = key.extra.start {
                if self.take(start.item) {
                    cues.push(Cue::Stop(start.item));
                }
                cues.push(Cue::Start(start.item));
                self.playing.push(start.item);
            }
        }
    }

    /// `item` no longer playing; whether it was.
    fn take(&mut self, item: Uuid) -> bool {
        let before = self.playing.len();
        self.playing.retain(|playing| *playing != item);
        self.playing.len() != before
    }
}

/// Where the animation started at keyframe `index` stops: the first later
/// keyframe that stops it, before one that starts it again; `None`: at the
/// end of the playback.
pub fn stops_at(track: &Track, index: usize) -> Option<usize> {
    let item = track.keys().get(index)?.extra.start?.item;
    for (later, key) in track.keys().iter().enumerate().skip(index + 1) {
        if key.extra.stop.is_some_and(|stop| stop.item == item) {
            return Some(later);
        }
        if key.extra.start.is_some_and(|start| start.item == item) {
            return None;
        }
    }
    None
}

/// The animation started at keyframe `index` stops at keyframe `until`
/// (`None`: at the end): the stops of it after `index`, up to the next
/// keyframe that starts it again, give way to that one.
pub fn set_stop(track: &mut Track, index: usize, until: Option<usize>) {
    let Some(anim) = track.keys().get(index).and_then(|key| key.extra.start) else {
        return;
    };
    let len = track.len();
    for later in index + 1..len {
        let mut extra = track.keys()[later].extra;
        if extra.stop.is_some_and(|stop| stop.item == anim.item) {
            extra.stop = None;
        }
        if Some(later) == until {
            extra.stop = Some(anim);
        }
        track.set_extra(later, extra);
        if Some(later) != until && extra.start.is_some_and(|start| start.item == anim.item) {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::Pose;
    use crate::director_keys::{AnimRef, KeyExtra};
    use glam::Vec3;

    fn anim(n: u128) -> AnimRef {
        AnimRef::new(Uuid::from_u128(n), &format!("Dance {n}"))
    }

    fn item(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// Five keyframes 3 s apart: the first starts dance 1, the second
    /// starts dance 2, the fourth stops dance 1.
    fn track() -> Director {
        let mut d = Director::default();
        for x in 0..5 {
            d.add_key(Pose::from_view(Vec3::new(x as f32, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0));
        }
        d.start_delay = 1.0;
        let set = |d: &mut Director, index: usize, extra: KeyExtra| d.track.set_extra(index, extra);
        set(
            &mut d,
            0,
            KeyExtra {
                start: Some(anim(1)),
                ..Default::default()
            },
        );
        set(
            &mut d,
            1,
            KeyExtra {
                start: Some(anim(2)),
                ..Default::default()
            },
        );
        set(
            &mut d,
            3,
            KeyExtra {
                stop: Some(anim(1)),
                ..Default::default()
            },
        );
        d
    }

    /// The cues of `seconds` of playback, a frame at a time.
    fn run(d: &mut Director, s: &mut Schedule, seconds: f32) -> Vec<Cue> {
        let mut cues = Vec::new();
        for _ in 0..(seconds * 10.0).round() as usize {
            d.tick(0.1);
            cues.extend(s.update(d));
        }
        cues
    }

    #[test]
    fn the_playback_starts_and_stops_the_animations_of_its_keyframes() {
        let (mut d, mut s) = (track(), Schedule::default());
        d.time = 0.0;
        assert!(d.play());
        // the start delay: the first view held, nothing plays yet
        assert!(s.update(&d).is_empty());
        assert!(run(&mut d, &mut s, 0.5).is_empty());
        // the camera leaves: the first keyframe's dance starts with it
        assert_eq!(run(&mut d, &mut s, 0.6), [Cue::Start(item(1))]);
        assert_eq!(run(&mut d, &mut s, 4.5), [Cue::Start(item(2))]);
        assert_eq!(run(&mut d, &mut s, 6.0), [Cue::Stop(item(1))]);
        assert_eq!(s.playing(), [item(2)]);
        // the end, then the hold on the last view: dance 2 plays on until
        // the playback is over
        let rest = run(&mut d, &mut s, 4.0);
        assert_eq!(rest, [Cue::Stop(item(2))]);
        assert_eq!(d.mode, Mode::Editing);
        assert!(s.playing().is_empty());
    }

    #[test]
    fn escape_or_a_teleport_stops_what_the_track_started() {
        let (mut d, mut s) = (track(), Schedule::default());
        d.time = 0.0;
        d.start_delay = 0.0;
        d.play();
        run(&mut d, &mut s, 4.0);
        assert_eq!(s.playing(), [item(1), item(2)]);
        // (the app stops the playback on Escape, a teleport, the window closed)
        d.stop();
        assert_eq!(s.update(&d), [Cue::Stop(item(2)), Cue::Stop(item(1))]);
        assert!(s.update(&d).is_empty(), "once");
    }

    #[test]
    fn a_playback_from_the_middle_plays_what_the_keyframes_left_playing() {
        let (mut d, mut s) = (track(), Schedule::default());
        d.start_delay = 0.0;
        // held on the timeline: no animation for a scrub
        d.scrub(7.0);
        assert!(s.update(&d).is_empty());
        d.end_scrub();
        d.play();
        // at 7 s: dance 1 and dance 2 started, neither stopped yet
        assert_eq!(s.update(&d), [Cue::Start(item(1)), Cue::Start(item(2))]);
        d.stop();
        s.update(&d);
        // at 10 s: dance 1 stopped at 9 s, so only dance 2
        d.time = 10.0;
        d.play();
        assert_eq!(s.update(&d), [Cue::Start(item(2))]);
    }

    #[test]
    fn a_loop_stops_everything_at_its_end_and_starts_over() {
        let (mut d, mut s) = (track(), Schedule::default());
        d.start_delay = 0.0;
        d.looping = true;
        d.time = 11.0;
        d.play();
        assert_eq!(s.update(&d), [Cue::Start(item(2))]);
        // past the end (12 s) and back at 0.5 s: dance 2 stops, dance 1
        // (first keyframe) starts again
        let cues = run(&mut d, &mut s, 1.5);
        assert_eq!(cues, [Cue::Stop(item(2)), Cue::Start(item(1))]);
        assert!(d.playing());
    }

    #[test]
    fn passing_a_keyframe_that_starts_a_playing_animation_starts_it_over() {
        let (mut d, mut s) = (track(), Schedule::default());
        d.track.set_extra(
            2,
            KeyExtra {
                start: Some(anim(2)),
                ..Default::default()
            },
        );
        d.start_delay = 0.0;
        d.time = 4.0;
        d.play();
        s.update(&d);
        assert_eq!(run(&mut d, &mut s, 2.5), [Cue::Stop(item(2)), Cue::Start(item(2))]);
        assert_eq!(s.playing(), [item(1), item(2)]);
    }

    #[test]
    fn a_stop_is_set_at_a_later_keyframe_or_at_the_end() {
        let mut d = track();
        assert_eq!(stops_at(&d.track, 0), Some(3));
        assert_eq!(stops_at(&d.track, 1), None);
        assert_eq!(stops_at(&d.track, 2), None, "no animation there");
        // dance 1 now stops at the third keyframe: the fourth's stop goes
        set_stop(&mut d.track, 0, Some(2));
        assert_eq!(stops_at(&d.track, 0), Some(2));
        assert_eq!(d.track.keys()[3].extra.stop, None);
        assert_eq!(d.track.keys()[1].extra.start, Some(anim(2)), "the others kept");
        // at the end
        set_stop(&mut d.track, 0, None);
        assert_eq!(stops_at(&d.track, 0), None);
        assert!(d.track.keys().iter().all(|key| key.extra.stop.is_none()));
        // a start of the same dance later bounds what is cleared
        d.track.set_extra(
            2,
            KeyExtra {
                start: Some(anim(1)),
                ..Default::default()
            },
        );
        set_stop(&mut d.track, 2, Some(4));
        set_stop(&mut d.track, 0, Some(1));
        assert_eq!((stops_at(&d.track, 0), stops_at(&d.track, 2)), (Some(1), Some(4)));
    }
}
