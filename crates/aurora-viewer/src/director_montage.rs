//! A montage played: the shots (saved tracks) one after the other, cut from
//! the last view of one to the first view of the next, as one playback and
//! so as one film. Aurora's own: the Legacy Viewer has no camera path tool.
//!
//! Each shot is played by the Director's own playback (`Director::tick`,
//! `view`): the player only puts the shot's track in the Director's place,
//! and at its end the next one, carrying the time left of the frame over the
//! cut so that the clock of the playback (and of a film, whose frames are
//! paced by the real clock, `recorder::Pace`) never stops. The start delay
//! is the first shot's only; the hold on the last view, the last shot's.
//! The track being edited, its selection and its playhead come back when the
//! montage ends, however it ends.

use crate::camera::track::Track;
use crate::director::{Director, Mode};

/// Frames a montage waits for its playback to start (a film's first frame
/// is the next one) before it gives up.
const START_FRAMES: u32 = 30;

/// What the Director had before the montage took its place.
#[derive(Debug, Clone, PartialEq)]
struct Kept {
    track: Track,
    time: f32,
    selected: Option<usize>,
    looping: bool,
}

/// What a frame of the montage did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The same shot goes on.
    Same,
    /// Cut to this shot (index among those that play).
    Cut(usize),
    /// The montage is over: the track being edited is back.
    Ended,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MontagePlayer {
    /// The shots that play, by name, in order.
    shots: Vec<(String, Track)>,
    current: usize,
    kept: Kept,
    /// The playback has started (it may wait a frame for a film).
    started: bool,
    waited: u32,
}

impl MontagePlayer {
    /// Put the first shot that plays in the Director's place (the others
    /// wait): `None` when none plays (each needs two keyframes). The playback
    /// itself is started by the caller (`Director::play`, or a film).
    pub fn start(d: &mut Director, shots: Vec<(String, Track)>) -> Option<MontagePlayer> {
        let shots: Vec<(String, Track)> = shots.into_iter().filter(|(_, track)| track.len() >= 2).collect();
        let first = shots.first()?.1.clone();
        let kept = Kept {
            track: d.track.clone(),
            time: d.time,
            selected: d.selected,
            looping: d.looping,
        };
        d.load(first);
        d.looping = false;
        Some(MontagePlayer {
            shots,
            current: 0,
            kept,
            started: false,
            waited: 0,
        })
    }

    /// The shot playing: its place among those that play, and its name.
    pub fn current(&self) -> (usize, &str) {
        (self.current, self.shots[self.current].0.as_str())
    }

    pub fn len(&self) -> usize {
        self.shots.len()
    }

    /// Seconds the montage lasts, shot after shot (no start delay, no hold).
    pub fn duration(&self) -> f32 {
        self.shots.iter().map(|(_, track)| track.duration()).sum()
    }

    /// After `Director::tick(dt)` (`before`: the playhead before it): at the
    /// end of a shot that has a next one, the cut; once the playback is over
    /// (or stopped: Escape, a teleport, the window closed), the track being
    /// edited back in its place.
    pub fn after_tick(&mut self, d: &mut Director, before: f32, dt: f32) -> Step {
        if !d.playing() {
            if !self.started && self.waited < START_FRAMES {
                self.waited += 1;
                return Step::Same;
            }
            self.finish(d);
            return Step::Ended;
        }
        self.started = true;
        let ended_shot = matches!(d.mode, Mode::Playing { hold: Some(_), .. });
        if !ended_shot || self.current + 1 >= self.shots.len() {
            return Step::Same;
        }
        // what the frame had left past the end of the shot plays in the next
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let over = (before + dt - d.track.duration()).max(0.0);
        self.current += 1;
        let next = self.shots[self.current].1.clone();
        d.time = over.min(next.duration());
        d.track = next;
        d.selected = None;
        d.mode = Mode::Playing { wait: 0.0, hold: None };
        Step::Cut(self.current)
    }

    /// The track being edited back, as it was; the Director edits again.
    pub fn finish(&self, d: &mut Director) {
        d.track = self.kept.track.clone();
        d.time = self.kept.time;
        d.selected = self.kept.selected.filter(|index| *index < d.track.len());
        d.looping = self.kept.looping;
        d.mode = Mode::Editing;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::track::Pose;
    use glam::Vec3;

    fn shot(keys: usize, x: f32) -> Track {
        let mut track = Track::default();
        for i in 0..keys {
            track.append(Pose::from_view(Vec3::new(x + i as f32, 0.0, 20.0), Vec3::X, Vec3::Z, 1.0));
        }
        track
    }

    fn edited() -> Director {
        Director {
            track: shot(3, 100.0),
            time: 1.5,
            selected: Some(1),
            looping: true,
            start_delay: 0.0,
            ..Default::default()
        }
    }

    /// Frames of `dt` until the montage ends: the shots seen, with the
    /// playhead at each cut, and the seconds played.
    fn run(d: &mut Director, player: &mut MontagePlayer, dt: f32) -> (Vec<(usize, f32)>, f32) {
        let (mut cuts, mut played) = (Vec::new(), 0.0);
        for _ in 0..10_000 {
            let before = d.time;
            let running = matches!(d.mode, Mode::Playing { hold: None, wait } if wait <= 0.0);
            d.tick(dt);
            if running {
                played += dt;
            }
            match player.after_tick(d, before, dt) {
                Step::Cut(index) => cuts.push((index, d.time)),
                Step::Ended => return (cuts, played),
                Step::Same => {}
            }
        }
        panic!("the montage never ended");
    }

    #[test]
    fn the_shots_play_one_after_the_other_and_the_edited_track_comes_back() {
        let mut d = edited();
        let kept = d.clone();
        // 6 s, then a shot of one keyframe (skipped), then 3 s
        let shots = vec![
            ("A".to_owned(), shot(3, 0.0)),
            ("One".to_owned(), shot(1, 10.0)),
            ("B".to_owned(), shot(2, 20.0)),
        ];
        let mut player = MontagePlayer::start(&mut d, shots).expect("a montage");
        assert_eq!((player.len(), player.duration()), (2, 9.0));
        assert_eq!(player.current(), (0, "A"));
        assert_eq!(d.track, shot(3, 0.0));
        assert!(!d.looping, "a montage plays once");
        assert!(d.play());
        let (cuts, played) = run(&mut d, &mut player, 0.25);
        assert_eq!(cuts, [(1, 0.0)]);
        // the film's clock never stopped at the cut: 6 s + 3 s of playback
        assert!((played - 9.0).abs() < 0.3, "{played}");
        assert_eq!(d.track, kept.track);
        assert_eq!((d.time, d.selected, d.looping, d.mode), (1.5, Some(1), true, Mode::Editing));
    }

    #[test]
    fn the_frame_left_over_at_a_cut_plays_in_the_next_shot() {
        let mut d = edited();
        let mut player =
            MontagePlayer::start(&mut d, vec![("A".to_owned(), shot(2, 0.0)), ("B".to_owned(), shot(3, 9.0))]).expect("montage");
        assert!(d.play());
        d.time = 2.9;
        let before = d.time;
        d.tick(0.3);
        assert_eq!(player.after_tick(&mut d, before, 0.3), Step::Cut(1));
        assert!((d.time - 0.2).abs() < 1e-4, "{}", d.time);
        assert_eq!(
            d.mode,
            Mode::Playing { wait: 0.0, hold: None },
            "no hold, no start delay between shots"
        );
        assert_eq!(player.current(), (1, "B"));
        // the view is the second shot's
        let view = d.view(|_| None).expect("view");
        assert!(view.position.x >= 9.0);
    }

    #[test]
    fn a_stopped_montage_gives_the_track_back() {
        let mut d = edited();
        let kept = d.track.clone();
        let mut player =
            MontagePlayer::start(&mut d, vec![("A".to_owned(), shot(2, 0.0)), ("B".to_owned(), shot(2, 5.0))]).expect("montage");
        d.play();
        d.tick(1.0);
        assert_eq!(player.after_tick(&mut d, 0.0, 1.0), Step::Same);
        // Escape
        d.stop();
        assert_eq!(player.after_tick(&mut d, 1.0, 0.0), Step::Ended);
        assert_eq!(d.track, kept);
        assert_eq!(d.mode, Mode::Editing);
    }

    #[test]
    fn a_montage_waits_for_its_film_then_gives_up() {
        let mut d = edited();
        let kept = d.track.clone();
        let mut player = MontagePlayer::start(&mut d, vec![("A".to_owned(), shot(2, 0.0))]).expect("montage");
        // the film's first frame is the next one: not played yet
        assert_eq!(player.after_tick(&mut d, 0.0, 0.016), Step::Same);
        // the film could not start: the montage does not stay forever
        let mut step = Step::Same;
        for _ in 0..START_FRAMES {
            step = player.after_tick(&mut d, 0.0, 0.016);
        }
        assert_eq!(step, Step::Ended);
        assert_eq!(d.track, kept);
    }

    #[test]
    fn a_montage_without_a_shot_that_plays_does_not_start() {
        let mut d = edited();
        let before = d.clone();
        assert!(MontagePlayer::start(&mut d, Vec::new()).is_none());
        assert!(MontagePlayer::start(&mut d, vec![("One".to_owned(), shot(1, 0.0))]).is_none());
        assert_eq!(d, before, "nothing changed");
    }

    #[test]
    fn the_last_shot_holds_its_view_then_the_montage_ends() {
        let mut d = edited();
        let mut player = MontagePlayer::start(&mut d, vec![("A".to_owned(), shot(2, 0.0))]).expect("montage");
        d.play();
        d.tick(5.0);
        assert_eq!(player.after_tick(&mut d, 0.0, 5.0), Step::Same);
        assert!(matches!(d.mode, Mode::Playing { hold: Some(_), .. }), "the hold of the last view");
        d.tick(crate::director::END_HOLD + 0.1);
        assert_eq!(player.after_tick(&mut d, 3.0, 1.1), Step::Ended);
    }
}
