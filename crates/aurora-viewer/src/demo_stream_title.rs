//! AURORA_DEMO_STREAM_TITLE=[window|history|text][,stop]: the parcel's
//! radio changes title every few seconds (one title too long for the
//! window, which scrolls), with four titles already in the history, and the
//! stream title window opens: alone, with its history, or as its text
//! alone laid over the world. `stop` ends the stream after the last title
//! ("No stream playing", the history kept).

/// What the scenario shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scenario {
    pub history: bool,
    pub text_only: bool,
    pub stop: bool,
}

pub fn scenario() -> Option<Scenario> {
    let mode = std::env::var("AURORA_DEMO_STREAM_TITLE").ok()?;
    let words: Vec<String> = mode.split(',').map(|w| w.trim().to_ascii_lowercase()).collect();
    let has = |w: &str| words.iter().any(|x| x == w);
    Some(Scenario {
        history: has("history"),
        text_only: has("text"),
        stop: has("stop"),
    })
}

/// The frame the window opens at, after the loading fade.
pub const OPEN: u64 = 600;

/// Frames between two titles (about 6 s at 60 fps).
const EVERY: u64 = 360;

/// Played before the window opens, then one after the other.
const TITLES: &[&str] = &[
    "Nova Lumen - Northern Lights",
    "The Quiet Harbour - Paper Boats",
    "Maple & Pine - Slow Morning",
    "Kite Season - Open Skies",
    "Aurora Ensemble feat. The Midnight Choir of the Northern Fjords - A Very Long Song Title That Has To Scroll (Extended Radio Edit)",
    "Low Tide - Lanterns",
    "Velvet Static - Night Drive",
];

/// The title the radio plays at `frame`: the first four are played before
/// the window opens, so the history is not empty; empty once stopped.
pub fn title(frame: u64, scenario: Scenario) -> &'static str {
    let first_shown = 4;
    if frame < OPEN {
        // spread over the loading, so they all reach the history
        let i = (frame * (first_shown as u64 + 1) / OPEN) as usize;
        return TITLES[i.min(first_shown)];
    }
    let i = first_shown + ((frame - OPEN) / EVERY) as usize;
    match TITLES.get(i) {
        Some(t) => t,
        None if scenario.stop => "",
        None => TITLES[first_shown + (i - first_shown) % (TITLES.len() - first_shown)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_fill_the_history_then_change_and_stop() {
        let s = Scenario {
            history: false,
            text_only: false,
            stop: true,
        };
        assert_eq!(title(0, s), TITLES[0]);
        assert_eq!(title(OPEN - 1, s), TITLES[4]);
        assert_eq!(title(OPEN + EVERY, s), TITLES[5]);
        assert_eq!(title(OPEN + 10 * EVERY, s), "");
        let looping = Scenario { stop: false, ..s };
        assert_eq!(title(OPEN + 3 * EVERY, looping), TITLES[4]);
    }
}
