//! The spinning cat: an easter egg of Aurora's own. Typing the Konami code
//! in the world brings a warning about flashing lights; once it is
//! accepted, a circle wakes on the ground in front of the avatar, a cat
//! arrives in it and spins to its music while the view takes the colours of
//! the beat, until the music ends or Stop (or Escape) is pressed.
//!
//! The music is "OIIA OIIA (Spinning Cat)" by W&W, played by YouTube's own
//! player in the viewer's video window, in plain sight: the viewer holds
//! none of it, only where its beats are (`video`), and follows where the
//! player says the video is. Where no video can play (no media plugins,
//! YouTube's previews hidden) the show does not start, and it ends if the
//! video does not.
//!
//! Everything else is local: the cat is drawn by this viewer only
//! (`scene/spinning_cat.rs`) and nothing is sent to the grid. The show
//! flashes, on purpose: it never starts by itself, the warning always
//! comes first and Cancel is its first answer. With the Reduce motion
//! setting the show is a calm one that does not flash (`show`).

pub mod konami;
pub mod show;
pub mod video;

use aurora_render::looks::{LookLayer, LookOp, PhotoLook};
use glam::Vec3;
use std::time::{Duration, Instant};

/// Metres in front of the avatar the circle wakes, and to its right.
pub const AHEAD: f32 = 2.2;
pub const ASIDE: f32 = 1.3;
/// How long the video may take to start before the show gives up.
const VIDEO_WAIT: Duration = Duration::from_secs(20);
/// Seconds the circle takes to wake while the video loads.
const WAKING: f32 = 2.5;

/// What the music of a show is followed on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The video, already asked of its player.
    Video,
    /// The video's timings on the clock, without its sound or its window:
    /// the demo's screenshots.
    Cues,
}

/// What the video's player last said, as the app reads it each frame.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct VideoNow {
    /// Seconds into the video, whether it plays, and how many seconds ago
    /// that was said; None until the player speaks.
    pub heard: Option<(f64, bool, f64)>,
    /// The player's window is open.
    pub open: bool,
}

enum Music {
    /// `started`: the player said the video plays.
    Video {
        asked: Instant,
        started: bool,
    },
    Cues,
}

struct Playing {
    music: Music,
    /// Seconds into the music as the show follows it: the frame's own
    /// clock, pulled to what is heard so that the picture moves evenly.
    clock: f64,
    /// Seconds since the show began, whatever the music does.
    elapsed: f64,
    centre: Vec3,
    facing: f32,
    /// The region the show started in: it ends with a teleport.
    region: Option<u64>,
}

/// What the scene draws this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stage {
    pub centre: Vec3,
    pub facing: f32,
    pub frame: show::Frame,
    pub time: f32,
}

#[derive(Default)]
pub struct Oiia {
    konami: konami::Konami,
    /// The warning is up, waiting for its answer.
    warning: bool,
    playing: Option<Playing>,
    stage: Option<Stage>,
    /// The video's window is to be closed: the show is over.
    close_video: bool,
}

/// The circle waking while the video loads: no cat yet.
fn waking(waited: f32) -> show::Moment {
    show::Moment {
        mood: show::Mood::Waking,
        through: (waited / WAKING).min(1.0),
        since: waited,
        left: f32::MAX,
        time: waited,
        beat: 0,
        beat_phase: 0.0,
        kick_age: f32::MAX,
        impact_age: f32::MAX,
        spin: None,
        phrase: 0,
        phrase_since: waited,
        phrase_left: f32::MAX,
        peak_since: None,
        arrived: -1.0,
        drops: 0,
        drop_ended: None,
    }
}

impl Oiia {
    /// A key went down in the world. `typing`: a text field has the
    /// keyboard, so the keys are text, not the code; but once the code's
    /// arrows were typed outside a field, its two letters count wherever
    /// they land: with the letters set to start the chat, B itself opens
    /// the chat bar before A is pressed.
    pub fn key(&mut self, key: konami::Key, now: Instant, typing: bool) {
        if self.warning || self.playing.is_some() || typing && self.konami.progress() < konami::ARROWS {
            self.konami.forget();
        } else if self.konami.press(key, now) {
            self.warning = true;
        }
    }

    pub fn warning(&self) -> bool {
        self.warning
    }

    /// The warning was answered without starting.
    pub fn decline(&mut self) {
        self.warning = false;
    }

    pub fn playing(&self) -> bool {
        self.playing.is_some()
    }

    /// Starts the show: the circle at `centre` (on the ground), the cat
    /// facing `facing`, the music followed on `source`, from its second
    /// `from` for the cues.
    pub fn start(&mut self, centre: Vec3, facing: f32, region: Option<u64>, source: Source, from: f64) {
        self.warning = false;
        self.close_video = false;
        self.playing = Some(Playing {
            music: match source {
                Source::Video => Music::Video {
                    asked: Instant::now(),
                    started: false,
                },
                Source::Cues => Music::Cues,
            },
            clock: from.max(0.0),
            elapsed: 0.0,
            centre,
            facing,
            region,
        });
    }

    pub fn stop(&mut self) {
        if matches!(self.playing.as_ref().map(|p| &p.music), Some(Music::Video { .. })) {
            self.close_video = true;
        }
        self.playing = None;
        self.stage = None;
    }

    /// True once when the video's window is to be closed.
    pub fn take_close_video(&mut self) -> bool {
        std::mem::take(&mut self.close_video)
    }

    /// Advances the show by a frame of `dt` seconds. `calm`: the Reduce
    /// motion setting. `region`: where the avatar is now. `video`: what
    /// the video's player says.
    pub fn tick(&mut self, dt: f32, calm: bool, region: Option<u64>, video: VideoNow) -> Option<Stage> {
        let playing = self.playing.as_mut()?;
        let dt = f64::from(dt.clamp(0.0, 0.25));
        playing.elapsed += dt;
        let moment = match &mut playing.music {
            Music::Cues => {
                playing.clock += dt;
                video::moment(playing.clock)
            }
            Music::Video { asked, started } => {
                let waited = asked.elapsed();
                // Closed by the resident: the show ends with its music.
                let closed = !video.open && (*started || waited > Duration::from_secs(1));
                match video.heard.filter(|(_, plays, _)| *plays || *started) {
                    _ if closed => None,
                    Some((seconds, plays, ago)) => {
                        let heard = seconds - f64::from(video::START) + if plays { ago } else { 0.0 };
                        if !*started {
                            *started = true;
                            playing.clock = heard.max(0.0);
                        } else if plays {
                            // Close a tenth of the gap to what is heard,
                            // which a frame's worth of sound never shows;
                            // far off (a hitch, a seek), go there.
                            playing.clock += dt;
                            let late = heard - playing.clock;
                            playing.clock += if late.abs() > 0.2 { late } else { late * 0.1 };
                        }
                        // Paused or loading: the show holds where it is.
                        video::moment(playing.clock.max(0.0))
                    }
                    // No network, a video that is gone: no show without
                    // its music.
                    None if waited > VIDEO_WAIT => None,
                    None => Some(waking(waited.as_secs_f32())),
                }
            }
        };
        let elapsed = playing.elapsed as f32;
        self.stage = moment.filter(|_| region == playing.region).map(|moment| Stage {
            centre: playing.centre,
            facing: playing.facing,
            // The circle turns and the other cats circle on the show's own
            // time: the music's may wait, jump or start late.
            frame: show::frame(&show::Moment { time: elapsed, ..moment }, calm),
            time: elapsed,
        });
        if self.stage.is_none() {
            self.stop();
        }
        self.stage
    }

    /// The look of the view for the last `tick`, over a frame of this size
    /// in pixels; None when the show leaves the view as it is.
    pub fn look(&self, width: u32, height: u32) -> Option<PhotoLook> {
        look(&self.stage?.frame, width, height)
    }
}

/// How much brighter than the edges the outlines are drawn.
const OUTLINE_GAIN: f32 = 3.0;
/// Height of the bands a glitch tears, as a share of the view.
const GLITCH_BAND: f32 = 0.05;
/// How bright a part of the view must be to glow.
const GLOW_THRESHOLD: f32 = 0.55;

fn look(frame: &show::Frame, width: u32, height: u32) -> Option<PhotoLook> {
    const LUMINOSITY: [f32; 3] = [0.2125, 0.7154, 0.0721];
    let layer = |ops: Vec<LookOp>, intensity: f32| LookLayer {
        ops: ops.into(),
        intensity,
        lut: None,
    };
    let mut layers = Vec::new();
    // What moves the picture: its bulge, its blocks, its torn bands, its
    // colours pulled apart.
    let mut warp = Vec::new();
    if frame.punch > 0.000_5 {
        warp.push(LookOp::Distort(frame.punch));
    }
    if frame.mosaic > 0.000_5 {
        warp.push(LookOp::Pixelate(frame.mosaic));
    }
    if frame.glitch > 0.000_5 {
        warp.push(LookOp::Glitch {
            amount: frame.glitch,
            band: GLITCH_BAND,
            seed: frame.glitch_seed,
        });
    }
    if frame.split > 0.000_5 {
        warp.push(LookOp::Chroma(frame.split));
    }
    if !warp.is_empty() {
        layers.push(layer(warp, 1.0));
    }
    // Its colours: turned around the wheel, tinted, read through a duotone.
    if frame.hue.abs() > 0.5 {
        layers.push(layer(vec![LookOp::Rotate(frame.hue)], 1.0));
    }
    if frame.tint_share > 0.003 {
        let [r, g, b] = frame.tint;
        layers.push(layer(
            vec![LookOp::ColorTransform([[r, 0.0, 0.0], [0.0, g, 0.0], [0.0, 0.0, b]])],
            frame.tint_share,
        ));
    }
    if frame.map_share > 0.003 {
        layers.push(layer(vec![LookOp::GradientMap(frame.map)], frame.map_share));
    }
    if frame.neon > 0.003 {
        // The edges of the picture, in one colour on black.
        let rows = frame.neon_color.map(|c| LUMINOSITY.map(|l| l * c * OUTLINE_GAIN));
        layers.push(layer(vec![LookOp::gradient(), LookOp::ColorTransform(rows)], frame.neon));
    }
    // The glow last, over what the layers before it made: the outlines
    // swell with its reach.
    if frame.bloom > 0.003 {
        layers.push(layer(
            vec![LookOp::Glow {
                amount: frame.bloom,
                radius: frame.bloom_radius,
                threshold: GLOW_THRESHOLD,
                tint: [1.0; 3],
            }],
            1.0,
        ));
    }
    (!layers.is_empty()).then_some(PhotoLook {
        layers,
        compare: None,
        image: [0.0, 0.0, width as f32, height as f32],
        scale: 1.0,
        hold: false,
        focus: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: [konami::Key; 10] = {
        use konami::Key::*;
        [Up, Up, Down, Down, Left, Right, Left, Right, B, A]
    };
    const QUIET: VideoNow = VideoNow { heard: None, open: false };
    const START: f64 = video::START as f64;

    fn typed(oiia: &mut Oiia, typing: bool) {
        let now = Instant::now();
        for (i, key) in CODE.into_iter().enumerate() {
            oiia.key(key, now + Duration::from_millis(100 * i as u64), typing);
        }
    }

    /// The player says the video is `seconds` in, playing, just now.
    fn at(seconds: f64) -> VideoNow {
        VideoNow {
            heard: Some((seconds, true, 0.0)),
            open: true,
        }
    }

    #[test]
    fn the_code_only_brings_the_warning_and_never_starts_the_show() {
        let mut oiia = Oiia::default();
        typed(&mut oiia, true);
        assert!(!oiia.warning(), "Typed in a text field: not the code");
        typed(&mut oiia, false);
        assert!(oiia.warning() && !oiia.playing());
        assert!(oiia.tick(0.016, false, None, QUIET).is_none());
        oiia.decline();
        assert!(!oiia.warning() && !oiia.playing());
    }

    #[test]
    fn the_circle_wakes_while_the_video_loads_then_the_show_follows_the_video() {
        let mut oiia = Oiia::default();
        oiia.start(Vec3::new(10.0, 20.0, 30.0), 1.0, None, Source::Video, 0.0);
        assert!(oiia.playing() && !oiia.warning());
        // Nothing heard yet: the circle, no cat.
        let waiting = oiia
            .tick(0.016, false, None, VideoNow { heard: None, open: true })
            .expect("waiting");
        assert_eq!((waiting.centre, waiting.facing), (Vec3::new(10.0, 20.0, 30.0), 1.0));
        assert!(waiting.frame.cat.is_none() && waiting.frame.circle > 0.0);
        assert_eq!(oiia.stage, Some(waiting));
        // The player is there but has not started: still waiting.
        let loading = VideoNow {
            heard: Some((START, false, 0.0)),
            open: true,
        };
        assert!(oiia.tick(0.016, false, None, loading).is_some_and(|s| s.frame.cat.is_none()));
        // The code typed during the show does nothing.
        typed(&mut oiia, false);
        assert!(!oiia.warning());
        // It plays: 4 s past the point the video is started at, the first
        // drop is on.
        let drop = oiia.tick(0.016, false, None, at(START + 4.0)).expect("the video plays");
        assert!(drop.frame.cat.is_some() && drop.frame.tint_share > 0.5);
        assert!((drop.time - 0.048).abs() < 1e-4, "The show's own time");
        // A report 0.1 s old counts for 0.1 s more.
        let later = VideoNow {
            heard: Some((START + 4.0, true, 0.1)),
            open: true,
        };
        oiia.tick(0.1, false, None, later).expect("still playing");
        assert!((oiia.playing.as_ref().unwrap().clock - 4.1).abs() < 0.02);
        // Paused: the show holds.
        let paused = VideoNow {
            heard: Some((START + 4.1, false, 0.0)),
            open: true,
        };
        oiia.tick(0.2, false, None, paused).expect("paused, not over");
        assert!((oiia.playing.as_ref().unwrap().clock - 4.1).abs() < 0.02);
        // Its window closed: the show ends, and nothing is left to close.
        assert!(oiia.tick(0.016, false, None, VideoNow { open: false, ..paused }).is_none());
        assert!(!oiia.playing() && oiia.stage.is_none() && oiia.look(1280, 720).is_none());
        assert!(oiia.take_close_video() && !oiia.take_close_video());
    }

    #[test]
    fn the_video_ends_the_show_and_its_window_is_closed() {
        let mut oiia = Oiia::default();
        oiia.start(Vec3::ZERO, 0.0, None, Source::Video, 0.0);
        assert!(oiia.tick(0.016, false, None, at(START + 50.0)).is_some());
        assert!(oiia.tick(0.016, false, None, at(START + video::LENGTH + 0.5)).is_none());
        assert!(oiia.take_close_video());
        // Stop closes it too.
        oiia.start(Vec3::ZERO, 0.0, None, Source::Video, 0.0);
        oiia.stop();
        assert!(oiia.take_close_video());
    }

    #[test]
    fn a_teleport_ends_the_show() {
        let mut oiia = Oiia::default();
        oiia.start(Vec3::ZERO, 0.0, Some(7), Source::Cues, 50.0);
        assert!(oiia.tick(0.016, false, Some(7), QUIET).is_some());
        assert!(oiia.tick(0.016, false, Some(8), QUIET).is_none());
        assert!(!oiia.playing());
    }

    #[test]
    fn the_cues_run_on_the_clock_without_a_player_to_their_end() {
        let mut oiia = Oiia::default();
        oiia.start(Vec3::ZERO, 0.0, None, Source::Cues, 10.0);
        let stage = oiia.tick(0.05, false, None, QUIET).expect("the cues");
        assert!(stage.frame.cat.is_some() && stage.frame.tint_share > 0.5, "The first drop");
        assert!((oiia.playing.as_ref().unwrap().clock - 10.05).abs() < 1e-6);
        oiia.start(Vec3::ZERO, 0.0, None, Source::Cues, video::LENGTH - 0.05);
        assert!(oiia.tick(0.1, false, None, QUIET).is_none(), "Past the end of the music");
        assert!(!oiia.playing() && !oiia.take_close_video(), "The cues have no window");
    }

    #[test]
    fn the_look_of_the_drop_tints_the_view_and_the_calm_one_only_tints_it() {
        for calm in [false, true] {
            let mut oiia = Oiia::default();
            oiia.start(Vec3::ZERO, 0.0, None, Source::Cues, 3.64);
            oiia.tick(0.0, calm, None, QUIET).expect("the show is on");
            let look = oiia.look(1920, 1080).expect("the drop has a look");
            assert_eq!(look.image, [0.0, 0.0, 1920.0, 1080.0]);
            let ops: Vec<&LookOp> = look.layers.iter().flat_map(|l| l.ops.iter()).collect();
            assert!(ops.iter().any(|op| matches!(op, LookOp::ColorTransform(_))));
            let moves = ops.iter().any(|op| matches!(op, LookOp::Chroma(_) | LookOp::Distort(_)));
            assert_eq!(moves, !calm);
            assert!(look.layers.iter().all(|l| (0.0..=1.0).contains(&l.intensity)));
        }
        // At rest between the drops, once the tint has faded, the view is
        // left alone.
        let mut oiia = Oiia::default();
        oiia.start(Vec3::ZERO, 0.0, None, Source::Cues, 31.0);
        oiia.tick(0.0, false, None, QUIET);
        assert!(oiia.look(1920, 1080).is_none());
    }
}
