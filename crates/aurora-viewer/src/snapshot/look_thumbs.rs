//! Which of the photo filters' catalogue pictures to draw, and when to take
//! a new still of the scene for them (`aurora_render::look_thumbs`; Aurora's
//! own): while tiles are shown, a still as soon as they show and every
//! `REFRESH` seconds, then the tiles shown drawn on it one a frame, each
//! picture kept until its new one is drawn (nothing flickers). A look whose
//! settings changed is drawn again. Nothing runs while no tile is shown.

use std::collections::{BTreeMap, HashMap, HashSet};

/// Seconds between two stills while the catalogue shows.
pub const REFRESH: f64 = 3.0;
/// Seconds without a tile shown after which the pictures are forgotten:
/// the renderer lets them go a while later (`aurora_render::look_thumbs`),
/// and they are all drawn again on a new still when the catalogue returns.
const FORGET: f64 = 2.0;

/// A look's picture: its slot among the renderer's, the still it was drawn
/// on (0: to draw again over the old one), its settings then, when it was
/// last shown, and whether the slot holds it (not when its look could not
/// be made: a table not read yet, a broken file; tried again on the next
/// still).
#[derive(Debug, Clone)]
struct Slot {
    index: usize,
    still: u64,
    settings: Option<Vec<f32>>,
    shown: f64,
    drawn: bool,
}

/// What to ask of the frame.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Take a new still.
    pub capture: bool,
    /// Draw this look into this slot.
    pub draw: Option<(usize, String)>,
}

#[derive(Debug, Default)]
pub struct Thumbs {
    slots: HashMap<String, Slot>,
    /// Stills taken (0: none yet).
    still: u64,
    last_capture: Option<f64>,
    /// When tiles were last shown.
    last_shown: Option<f64>,
    /// The part of the screen the stills are taken from (pixels).
    source: Option<[u32; 4]>,
}

impl Thumbs {
    /// The part of the screen the stills come from: the photo's frame, the
    /// Director's picture, or the whole 3D view for the World filters
    /// window alone. Changed (a window opened or closed, the format or the
    /// window's size changed), a new still is taken at once instead of the
    /// tiles keeping the old crop until the next `REFRESH`.
    pub fn source(&mut self, rect: [u32; 4]) {
        if self.source.replace(rect).is_some_and(|old| old != rect) {
            self.last_capture = None;
        }
    }

    /// This frame's request for the tiles `shown`, with `capacity` slots
    /// (the renderer makes them on the first request) and the looks'
    /// `settings`.
    pub fn plan(&mut self, now: f64, shown: &[String], capacity: usize, settings: &BTreeMap<String, Vec<f32>>) -> Plan {
        if shown.is_empty() || capacity == 0 {
            return Plan::default();
        }
        if self.last_shown.is_some_and(|t| now - t > FORGET) {
            self.slots.clear();
            self.last_capture = None;
        }
        self.last_shown = Some(now);
        let capture = self.last_capture.is_none_or(|t| now - t >= REFRESH);
        if capture {
            self.still += 1;
            self.last_capture = Some(now);
        }
        for id in shown {
            if let Some(slot) = self.slots.get_mut(id) {
                slot.shown = now;
            }
        }
        let still = self.still;
        let next = shown.iter().find(|id| {
            self.slots
                .get(*id)
                .is_none_or(|s| s.still != still || s.settings.as_ref() != settings.get(*id))
        });
        let draw = next.and_then(|id| {
            let index = match self.slots.get(id) {
                Some(slot) => slot.index,
                None => self.free_slot(capacity, shown)?,
            };
            self.slots.insert(
                id.clone(),
                Slot {
                    index,
                    still,
                    settings: settings.get(id).cloned(),
                    shown: now,
                    drawn: true,
                },
            );
            Some((index, id.clone()))
        });
        Plan { capture, draw }
    }

    /// A slot no picture holds, or the one of the picture shown longest
    /// ago among those not shown now.
    fn free_slot(&mut self, capacity: usize, shown: &[String]) -> Option<usize> {
        let used: HashSet<usize> = self.slots.values().map(|s| s.index).collect();
        if let Some(i) = (0..capacity).find(|i| !used.contains(i)) {
            return Some(i);
        }
        let (id, index) = self
            .slots
            .iter()
            .filter(|(id, _)| !shown.contains(id))
            .min_by(|a, b| a.1.shown.total_cmp(&b.1.shown))
            .map(|(id, s)| (id.clone(), s.index))?;
        self.slots.remove(&id);
        Some(index)
    }

    /// The look planned this frame could not be drawn: its tile shows no
    /// picture (its slot may hold another look's) until the next still.
    pub fn failed(&mut self, id: &str) {
        if let Some(slot) = self.slots.get_mut(id) {
            slot.drawn = false;
        }
    }

    /// The slot of each look's picture drawn so far.
    pub fn slots(&self) -> HashMap<String, usize> {
        self.slots
            .iter()
            .filter(|(_, s)| s.drawn)
            .map(|(id, s)| (id.clone(), s.index))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A look that could not be made (a table not read yet) shows no
    /// picture, even in a slot another look's picture held, does not hold
    /// up the others, and is tried again on the next still.
    #[test]
    fn a_look_not_drawn_shows_no_picture_until_the_next_still() {
        let mut t = Thumbs::default();
        let none = BTreeMap::new();
        let shown = ids(&["lut:a", "Sepia"]);
        assert_eq!(t.plan(0.0, &shown, 4, &none).draw, Some((0, "lut:a".to_owned())));
        t.failed("lut:a");
        assert!(!t.slots().contains_key("lut:a"));
        assert_eq!(
            t.plan(0.02, &shown, 4, &none).draw,
            Some((1, "Sepia".to_owned())),
            "the next one is drawn"
        );
        assert_eq!(t.plan(0.04, &shown, 4, &none).draw, None, "not again before the still");
        let p = t.plan(REFRESH + 0.1, &shown, 4, &none);
        assert!(p.capture);
        assert_eq!(p.draw, Some((0, "lut:a".to_owned())), "tried again on the new still");
        assert!(t.slots().contains_key("lut:a"));
    }

    /// A new frame to take the stills from (the photo window closed with
    /// the World filters window open, a window resized) takes a still at
    /// once, and the tiles are drawn on it again.
    #[test]
    fn a_new_source_takes_a_still_at_once() {
        let mut t = Thumbs::default();
        let none = BTreeMap::new();
        let shown = ids(&["Sepia"]);
        t.source([100, 50, 900, 650]);
        assert!(t.plan(0.0, &shown, 4, &none).capture);
        assert!(!t.plan(0.5, &shown, 4, &none).capture);
        t.source([100, 50, 900, 650]);
        assert!(!t.plan(1.0, &shown, 4, &none).capture, "the same frame: no new still");
        t.source([0, 0, 1280, 900]);
        let p = t.plan(1.5, &shown, 4, &none);
        assert!(p.capture, "the whole view now: a new still");
        assert_eq!(p.draw, Some((0, "Sepia".to_owned())), "drawn again on it");
    }

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    #[test]
    fn a_still_first_then_one_picture_a_frame() {
        let mut t = Thumbs::default();
        let none = BTreeMap::new();
        let shown = ids(&["Sepia", "aurora:vhs"]);
        // the first frame takes a still and draws on it at once
        assert_eq!(
            t.plan(0.0, &shown, 4, &none),
            Plan {
                capture: true,
                draw: Some((0, "Sepia".to_owned()))
            }
        );
        let p = t.plan(0.02, &shown, 4, &none);
        assert!(!p.capture, "the still was just taken");
        assert_eq!(p.draw, Some((1, "aurora:vhs".to_owned())));
        assert_eq!(t.plan(0.04, &shown, 4, &none), Plan::default(), "all drawn on this still");
        // frames go by: nothing more to do until the next still
        for frame in 1..=(REFRESH * 10.0) as usize {
            assert_eq!(t.plan(0.04 + frame as f64 / 10.0 - 0.05, &shown, 4, &none), Plan::default());
        }
        // a new still: each drawn again in its own slot, the old picture
        // shown until then
        let p = t.plan(REFRESH + 0.1, &shown, 4, &none);
        assert!(p.capture);
        assert_eq!(p.draw, Some((0, "Sepia".to_owned())));
        assert_eq!(t.slots().len(), 2);
    }

    #[test]
    fn nothing_runs_while_no_tile_shows() {
        let mut t = Thumbs::default();
        assert_eq!(t.plan(0.0, &[], 4, &BTreeMap::new()), Plan::default());
    }

    #[test]
    fn a_look_whose_settings_changed_is_drawn_again() {
        let mut t = Thumbs::default();
        let shown = ids(&["aurora:glitch"]);
        let mut settings = BTreeMap::new();
        t.plan(0.0, &shown, 2, &settings);
        assert_eq!(t.plan(0.1, &shown, 2, &settings), Plan::default());
        settings.insert("aurora:glitch".to_owned(), vec![0.4]);
        assert_eq!(t.plan(0.2, &shown, 2, &settings).draw, Some((0, "aurora:glitch".to_owned())));
    }

    #[test]
    fn a_full_set_of_slots_reuses_the_one_shown_longest_ago() {
        let mut t = Thumbs::default();
        let none = BTreeMap::new();
        t.plan(0.0, &ids(&["A"]), 2, &none);
        t.plan(1.0, &ids(&["B"]), 2, &none);
        // C needs a slot: A was shown longest ago and is not shown now
        assert_eq!(t.plan(1.5, &ids(&["C"]), 2, &none).draw, Some((0, "C".to_owned())));
        let slots = t.slots();
        assert!(!slots.contains_key("A") && slots.contains_key("B") && slots.contains_key("C"));
    }

    #[test]
    fn a_catalogue_hidden_a_while_draws_everything_again_on_a_new_still() {
        let mut t = Thumbs::default();
        let none = BTreeMap::new();
        t.plan(0.0, &ids(&["A"]), 2, &none);
        assert_eq!(t.plan(0.5, &ids(&["A"]), 2, &none), Plan::default());
        // hidden, then shown again after the renderer may have let go
        assert_eq!(
            t.plan(10.0, &ids(&["A"]), 2, &none),
            Plan {
                capture: true,
                draw: Some((0, "A".to_owned()))
            }
        );
    }
}
