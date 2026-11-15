//! Avatar marks: a colour the user gives a resident to follow them in a
//! crowd. The mark shows on every mini-map dot of theirs, on their name tag
//! and on the mouselook target arrows; the People lists ring their picture.
//! Port of LLNetMap's mark feature (sAvatarMarksMap, setAvatarMarkColors,
//! clearAvatarMarkColors, getAvatarColor in indra/newview/llnetmap.cpp,
//! originally LGPL 2.1): one map for the whole viewer, kept for the session
//! only, never saved nor sent. The World is made anew at each login, so the
//! marks go with the session, as the Legacy Viewer's static map goes with
//! its process.

use std::collections::HashMap;
use uuid::Uuid;

/// An sRGB colour, opaque.
pub type Rgb = [u8; 3];

/// The colours of the Mark menus (menu_fs_radar.xml, menu_mini_map.xml:
/// Red, Green, Blue, Purple and LtYellow of colors.xml), in their order.
pub const LEGACY_COLORS: [(MarkColor, Rgb); 5] = [
    (MarkColor::Red, [186, 0, 31]),
    (MarkColor::Green, [0, 255, 0]),
    (MarkColor::Blue, [0, 0, 255]),
    (MarkColor::Purple, [255, 0, 255]),
    (MarkColor::Yellow, [255, 255, 201]),
];

/// The names of the menu's colours, for their labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkColor {
    Red,
    Green,
    Blue,
    Purple,
    Yellow,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct AvatarMarks {
    marks: HashMap<Uuid, Rgb>,
}

impl AvatarMarks {
    /// The mark of a resident, if they have one.
    pub fn get(&self, id: &Uuid) -> Option<Rgb> {
        self.marks.get(id).copied()
    }

    pub fn contains(&self, id: &Uuid) -> bool {
        self.marks.contains_key(id)
    }

    pub fn is_empty(&self) -> bool {
        self.marks.is_empty()
    }

    /// Marks every resident of `ids` with `color`, replacing a mark they had
    /// (setAvatarMarkColors).
    pub fn set(&mut self, ids: &[Uuid], color: Rgb) {
        for id in ids {
            self.marks.insert(*id, color);
        }
    }

    /// "Clear Mark(s)": the marks of these residents (clearAvatarMarkColors).
    pub fn clear(&mut self, ids: &[Uuid]) {
        for id in ids {
            self.marks.remove(id);
        }
    }

    /// "Clear all Marks".
    pub fn clear_all(&mut self) {
        self.marks.clear();
    }

    /// The colour all of `ids` share, if they are all marked with it: what
    /// the custom colour window starts from.
    pub fn shared(&self, ids: &[Uuid]) -> Option<Rgb> {
        let first = self.get(ids.first()?)?;
        ids.iter().all(|id| self.get(id) == Some(first)).then_some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(1);
    const B: Uuid = Uuid::from_u128(2);
    const C: Uuid = Uuid::from_u128(3);

    #[test]
    fn a_mark_is_set_changed_and_shared_by_several() {
        let mut marks = AvatarMarks::default();
        assert!(marks.is_empty());
        let red = LEGACY_COLORS[0].1;
        let blue = LEGACY_COLORS[2].1;
        marks.set(&[A], red);
        assert_eq!(marks.get(&A), Some(red));
        assert!(!marks.contains(&B));
        // another colour replaces the first
        marks.set(&[A], blue);
        assert_eq!(marks.get(&A), Some(blue));
        // the multi-select menu marks every resident chosen
        marks.set(&[B, C], red);
        assert_eq!(marks.shared(&[B, C]), Some(red));
        assert_eq!(marks.shared(&[A, B]), None);
    }

    #[test]
    fn a_custom_colour_is_kept_as_chosen() {
        let mut marks = AvatarMarks::default();
        marks.set(&[A, B], [12, 34, 56]);
        assert_eq!(marks.get(&A), Some([12, 34, 56]));
        assert_eq!(marks.shared(&[A, B]), Some([12, 34, 56]));
    }

    #[test]
    fn clearing_one_leaves_the_others_and_clear_all_removes_every_mark() {
        let mut marks = AvatarMarks::default();
        marks.set(&[A, B, C], LEGACY_COLORS[1].1);
        marks.clear(&[B]);
        assert!(marks.contains(&A) && !marks.contains(&B) && marks.contains(&C));
        // clearing an unmarked resident does nothing
        marks.clear(&[B]);
        assert!(marks.contains(&A));
        marks.clear_all();
        assert!(marks.is_empty());
    }

    #[test]
    fn the_menu_offers_the_legacy_colours_in_their_order() {
        let names: Vec<MarkColor> = LEGACY_COLORS.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [MarkColor::Red, MarkColor::Green, MarkColor::Blue, MarkColor::Purple, MarkColor::Yellow]
        );
        // colors.xml: Red 0.729 0 0.121, LtYellow 1 1 .79
        assert_eq!(LEGACY_COLORS[0].1, [186, 0, 31]);
        assert_eq!(LEGACY_COLORS[4].1, [255, 255, 201]);
    }

    #[test]
    fn marks_last_for_the_session_only() {
        // a new World (each login) starts with none, and nothing of them is
        // serialisable to the settings
        let world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        assert!(world.avatar_marks.is_empty());
    }
}
