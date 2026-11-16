//! The cue sheet of the video the show plays its music from: where the
//! beats, the kicks and the drops of "OIIA OIIA (Spinning Cat)" by W&W are,
//! from the point the show starts the video at. Only timings are kept here;
//! the music itself is played by YouTube's own player, in a window of the
//! viewer (`ui/chat/previews.rs`), and nothing of it is in the viewer.
//!
//! The timings were measured on the track: 150 BPM exactly, its first beat
//! 0.24 s in, two drops of four phrases of sixteen beats, the last beat of
//! each phrase a fill without a kick, and two lone hits of bass in the
//! silence between the drops.

use super::show::{Moment, Mood, Spin};

/// The video on YouTube.
pub const VIDEO: &str = "IxX_QHay02M";
/// Seconds into the video the show starts it at: past its opening.
pub const START: u32 = 30;
/// Seconds of a beat: 150 BPM.
pub const BEAT: f64 = 0.4;
/// The first beat, in seconds from `START`.
const FIRST: f64 = 0.24;
/// The beat the show ends on.
const END: u32 = 256;
/// Seconds of the video the show plays.
pub const LENGTH: f64 = FIRST + END as f64 * BEAT;
/// Beats of a phrase: each phrase of a drop has its own look.
const PHRASE: u32 = 16;

/// The beats with a kick, first and last of each run.
const KICKS: [(u32, u32); 8] = [
    (8, 22),
    (24, 37),
    (40, 54),
    (56, 67),
    (192, 206),
    (208, 219),
    (224, 238),
    (240, 251),
];
/// The lone hits of bass in the silence between the drops.
const IMPACTS: [u32; 2] = [112, 128];

/// The music's parts: the beat each begins on, and what it does.
const PARTS: [(u32, Mood); 11] = [
    (0, Mood::Voice),
    (8, Mood::Drop),
    (68, Mood::Voice),
    (72, Mood::Still),
    (80, Mood::Sway),
    (112, Mood::Still),
    (144, Mood::Rise),
    (176, Mood::Hold),
    // The voice comes back alone, a bar and a half before the second drop.
    (184, Mood::Wait),
    (192, Mood::Drop),
    (252, Mood::Leave),
];
/// The beat the second drop begins on: the other cats come.
const PEAK: u32 = 192;

fn seconds_of(beat: u32) -> f64 {
    FIRST + f64::from(beat) * BEAT
}

/// Where the music is `seconds` after the point the video is started at;
/// None before it and once the show's part of it is over.
pub fn moment(seconds: f64) -> Option<Moment> {
    if !(0.0..LENGTH).contains(&seconds) {
        return None;
    }
    // Before the first beat the music has begun: it counts as that beat's.
    let beats = ((seconds - FIRST) / BEAT).max(0.0);
    let beat = beats as u32;
    let index = PARTS.iter().rposition(|(first, _)| *first <= beat)?;
    let (first, mood) = PARTS[index];
    let next = PARTS.get(index + 1).map_or(END, |(first, _)| *first);
    // Seconds from a beat to now, and from now to a beat.
    let from = |beat: u32| (seconds - seconds_of(beat)).max(0.0) as f32;
    let until = |beat: u32| (seconds_of(beat) - seconds).max(0.0) as f32;
    let since = from(first);
    let kicked = KICKS.iter().rev().find(|(from, _)| *from <= beat).map(|(_, to)| beat.min(*to));
    // A run of the spin over `beats` beats from the part's start.
    let run = |beats: u32, turns: f32| {
        let run = f64::from(beats) * BEAT;
        Spin {
            through: (f64::from(since) / run).fract() as f32,
            seconds: run as f32,
            turns,
        }
    };
    // The phrase of a drop; the whole part elsewhere.
    let (phrase, phrase_from, phrase_to) = if mood == Mood::Drop {
        let phrase = (beat - first) / PHRASE;
        (phrase, first + phrase * PHRASE, (first + (phrase + 1) * PHRASE).min(next))
    } else {
        (0, first, next)
    };
    Some(Moment {
        mood,
        through: since / (f64::from(next - first) * BEAT) as f32,
        since,
        left: until(next),
        time: seconds as f32,
        beat,
        beat_phase: beats.fract() as f32,
        kick_age: kicked.map_or(f32::MAX, from),
        impact_age: IMPACTS.iter().rev().find(|hit| **hit <= beat).map_or(f32::MAX, |hit| from(*hit)),
        spin: match mood {
            Mood::Voice | Mood::Wait | Mood::Drop => Some(run(1, 2.0)),
            Mood::Sway => Some(run(4, 1.0)),
            Mood::Rise => Some(run(4, 1.0 + ((beat - first) / 8) as f32)),
            Mood::Waking | Mood::Still | Mood::Hold | Mood::Leave => None,
        },
        phrase,
        phrase_since: from(phrase_from),
        phrase_left: until(phrase_to),
        peak_since: (beat >= PEAK).then(|| from(PEAK)),
        arrived: seconds as f32,
        drops: PARTS[..=index].iter().filter(|(_, mood)| *mood == Mood::Drop).count() as u32,
        // The last drop before this part ended where the part after it began.
        drop_ended: (mood != Mood::Drop)
            .then(|| PARTS[..index].iter().rposition(|(_, mood)| *mood == Mood::Drop))
            .flatten()
            .map(|drop| from(PARTS[drop + 1].0)),
    })
}

#[cfg(test)]
mod tests {
    use super::super::show;
    use super::*;

    #[test]
    fn the_parts_follow_the_track() {
        assert!(moment(-0.01).is_none() && moment(LENGTH).is_none());
        assert!((LENGTH - 102.64).abs() < 1e-9);
        // The voice alone, before the first beat and after it.
        let opening = moment(0.1).unwrap();
        assert_eq!((opening.mood, opening.beat, opening.drops), (Mood::Voice, 0, 0));
        assert!(opening.spin.is_some() && opening.kick_age == f32::MAX && (opening.arrived - 0.1).abs() < 1e-6);
        // The first drop begins on the eighth beat, 3.44 s in.
        let drop = moment(3.54).unwrap();
        assert_eq!((drop.mood, drop.beat, drop.drops, drop.phrase), (Mood::Drop, 8, 1, 0));
        assert!((drop.kick_age - 0.1).abs() < 1e-4 && (drop.since - 0.1).abs() < 1e-4 && drop.peak_since.is_none());
        assert!((drop.phrase_since - 0.1).abs() < 1e-4 && (drop.phrase_left - 6.3).abs() < 1e-4);
        assert!(drop.drop_ended.is_none() && opening.drop_ended.is_none());
        // The last beat of a phrase has no kick: the one before still rings.
        let fill = moment(seconds_of(23) + 0.1).unwrap();
        assert!((fill.kick_age - 0.5).abs() < 1e-4, "{}", fill.kick_age);
        // Its last phrase is the fourth, three bars long.
        let last = moment(seconds_of(60) + 0.1).unwrap();
        assert_eq!(last.phrase, 3);
        assert!((last.phrase_since - 1.7).abs() < 1e-4 && (last.phrase_left - 3.1).abs() < 1e-4);
        assert!((last.left - 3.1).abs() < 1e-4);
        // Between the drops the cat sways, a turn per bar; the drop ended
        // on beat 68, whatever part came since.
        let sway = moment(seconds_of(84) + 0.4).unwrap();
        assert_eq!((sway.mood, sway.drops), (Mood::Sway, 1));
        assert_eq!(sway.spin.map(|s| (s.seconds, s.turns)), Some((1.6, 1.0)));
        assert!((sway.spin.unwrap().through - 0.25).abs() < 1e-4);
        assert!((sway.drop_ended.unwrap() - 6.8).abs() < 1e-4);
        // The climb turns faster and faster, then holds its breath.
        let turns = |beat| moment(seconds_of(beat) + 0.1).unwrap().spin.unwrap().turns;
        assert_eq!((turns(144), turns(152), turns(168)), (1.0, 2.0, 4.0));
        let hold = moment(seconds_of(178)).unwrap();
        assert!(hold.mood == Mood::Hold && hold.spin.is_none());
    }

    #[test]
    fn the_hits_of_bass_in_the_silence_are_told() {
        assert_eq!(moment(seconds_of(100)).unwrap().impact_age, f32::MAX);
        let struck = moment(seconds_of(112) + 0.2).unwrap();
        assert_eq!(struck.mood, Mood::Still);
        assert!((struck.impact_age - 0.2).abs() < 1e-4 && struck.spin.is_none());
        assert!((moment(seconds_of(128) + 1.0).unwrap().impact_age - 1.0).abs() < 1e-4);
    }

    #[test]
    fn the_view_waits_for_the_second_drop_and_the_other_cats_come_with_it() {
        let waiting = moment(seconds_of(184) + 0.8).unwrap();
        assert_eq!(waiting.mood, Mood::Wait);
        assert!((waiting.since - 0.8).abs() < 1e-4 && (waiting.left - 2.4).abs() < 1e-4);
        assert!(show::frame(&waiting, false).neon > 0.8, "As outlines");
        let second = moment(seconds_of(192) + 0.2).unwrap();
        assert_eq!((second.mood, second.drops, second.phrase), (Mood::Drop, 2, 0));
        assert!(second.peak_since.is_some_and(|s| (s - 0.2).abs() < 1e-4));
        // The break in it: four beats without a kick.
        assert!(moment(seconds_of(222) + 0.2).unwrap().kick_age > 1.0);
        assert!(moment(seconds_of(224) + 0.1).unwrap().kick_age < 0.11);
        assert_eq!(moment(seconds_of(245)).unwrap().phrase, 3);
        let leaving = moment(seconds_of(252) + 1.5).unwrap();
        assert_eq!(leaving.mood, Mood::Leave);
        assert!(show::frame(&leaving, false).cat.is_none(), "The cat is gone before the end");
    }

    #[test]
    fn the_cat_keeps_its_size_between_the_parts() {
        for calm in [false, true] {
            let sizes: Vec<f32> = (0..)
                .map(|i| f64::from(i) / 240.0)
                .map_while(moment)
                .filter_map(|m| show::frame(&m, calm).cat)
                .map(|c| c.scale)
                .collect();
            let jump = sizes.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
            // Its size only jumps by the pop of a kick.
            assert!(jump < 0.1, "The cat's size jumps by {jump} (calm: {calm})");
        }
    }
}
