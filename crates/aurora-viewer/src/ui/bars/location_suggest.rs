//! What the location field suggests while a place is typed: the landmarks
//! whose name holds the text and the places of the teleport history whose
//! title does, in dictionary order, one of them chosen at once so that
//! Enter goes to it (a landmark to its exact position), and the typed text
//! taken as a SLURL or a region's name only when nothing matches.
//! Port of LLLocationInputCtrl::onLocationPrearrange,
//! findTeleportItemsByTitle and addLocationHistoryEntry
//! (indra/newview/lllocationinputctrl.cpp), LLComboBox::updateSelection
//! (indra/llui/llcombobox.cpp, with the substring search on by default),
//! LLNavigationBar::onLocationSelection (indra/newview/llnavigationbar.cpp)
//! and LLStringUtil::compareDict (indra/llcommon/llstring.h), originally
//! LGPL 2.1.

use super::BarAction;
use crate::theme::Palette;
use crate::ui::menu;
use crate::world::landmarks::Named;
use crate::world::tphistory::TpEntry;
use aurora_net::RegionHandle;
use egui::{Rect, Vec2};
use glam::Vec3;
use std::cmp::Ordering;
use uuid::Uuid;

/// Where a suggestion goes.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// TeleportLandmarkRequest with the landmark's asset
    /// (LLAgent::teleportViaLandmark).
    Landmark(Uuid),
    /// TeleportLocationRequest to a place of the teleport history
    /// (LLAgent::teleportViaLocation).
    Place {
        region: String,
        handle: RegionHandle,
        position: Vec3,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub label: String,
    pub target: Target,
}

impl Suggestion {
    pub fn action(&self) -> BarAction {
        match &self.target {
            Target::Landmark(asset) => BarAction::TeleportLandmark(*asset),
            Target::Place { region, handle, position } => BarAction::TeleportPlace {
                region: region.clone(),
                handle: *handle,
                position: *position,
            },
        }
    }

    fn icon(&self) -> &'static str {
        match self.target {
            Target::Landmark(_) => "map-pin-filled",
            Target::Place { .. } => "history",
        }
    }
}

/// The list for `typed`: the landmarks whose name holds it, whatever the
/// case, then the places of the history whose title holds it as typed
/// (findTeleportItemsByTitle is case-sensitive), one row per label
/// (SL-20286), sorted like the Legacy Viewer's list. Empty text, empty list.
pub fn suggestions(landmarks: &[Named], history: &[TpEntry], typed: &str) -> Vec<Suggestion> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Vec::new();
    }
    let lower = typed.to_lowercase();
    let mut out: Vec<Suggestion> = Vec::new();
    let mut push = |label: String, target: Target| {
        if !out.iter().any(|s| s.label == label) {
            out.push(Suggestion { label, target });
        }
    };
    for lm in landmarks.iter().filter(|lm| lm.lower.contains(&lower)) {
        push(lm.name.clone(), Target::Landmark(lm.asset));
    }
    for e in history {
        let title = e.title();
        if !title.is_empty() && title.contains(typed) {
            push(
                title,
                Target::Place {
                    region: e.region.clone(),
                    handle: e.handle,
                    position: e.position,
                },
            );
        }
    }
    // sortByName: a stable sort in dictionary order
    out.sort_by(|a, b| compare_dict(&a.label, &b.label));
    out
}

/// The row Enter takes at once: the one named exactly as typed, whatever
/// the case, else the first one whose label holds the text
/// (selectItemByLabel, then selectItemBySubstring).
pub fn pick(list: &[Suggestion], typed: &str) -> Option<usize> {
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return None;
    }
    list.iter()
        .position(|s| s.label.to_lowercase() == typed)
        .or_else(|| list.iter().position(|s| s.label.to_lowercase().contains(&typed)))
}

/// LLStringUtil::compareDict: case-insensitive, the first difference of
/// case breaking ties (upper case first), and runs of digits compared as
/// numbers ("Store 2" before "Store 10").
pub fn compare_dict(a: &str, b: &str) -> Ordering {
    fn lower(c: char) -> char {
        c.to_lowercase().next().unwrap_or(c)
    }
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let at = |s: &[char], i: usize| s.get(i).copied().unwrap_or('\0');
    let (mut i, mut j) = (1, 1);
    let (mut ca, mut cb) = (at(&a, 0), at(&b, 0));
    let mut bias: i64 = 0;
    // digits of the current run still to compare one by one
    let mut run: usize = 0;
    while ca != '\0' && cb != '\0' {
        if bias == 0 {
            if ca.is_uppercase() {
                ca = lower(ca);
                bias -= 1;
            }
            if cb.is_uppercase() {
                cb = lower(cb);
                bias += 1;
            }
        } else {
            ca = lower(ca);
            cb = lower(cb);
        }
        if ca.is_ascii_digit() {
            if run > 0 {
                run -= 1;
                if ca != cb {
                    break;
                }
            } else {
                if !cb.is_ascii_digit() {
                    break;
                }
                // the shorter run of digits is the smaller number
                let left = a[i..].iter().take_while(|c| c.is_ascii_digit()).count();
                let right = b[j..].iter().take_while(|c| c.is_ascii_digit()).count();
                match left.cmp(&right) {
                    Ordering::Less => return Ordering::Less,
                    Ordering::Greater => return Ordering::Greater,
                    Ordering::Equal => {}
                }
                if ca != cb {
                    break;
                }
                run = left;
            }
        } else if ca != cb {
            break;
        }
        ca = at(&a, i);
        cb = at(&b, j);
        i += 1;
        j += 1;
    }
    let tie = if ca == cb { bias } else { 0 };
    (ca as i64 + tie).cmp(&(cb as i64))
}

/// What `AURORA_DEMO_BARS=locations` types: the start of the demo's
/// "Aurora …" landmarks in every language, and of some places of its
/// history (matched as typed).
pub const DEMO_TYPED: &str = "Aur";

/// Frame the demo types it at (the place menu opens at frame 300).
pub const DEMO_FRAME: u64 = 320;

/// The arrows move the choice, Enter takes it (when the field has the
/// keyboard and the list rows). Read before the field, which would lose its
/// focus on Enter. Returns the row Enter took.
pub fn keys(ui: &egui::Ui, n: usize, chosen: &mut usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    ui.input_mut(|i| {
        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            *chosen = (*chosen + 1) % n;
        }
        if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            *chosen = (*chosen + n - 1) % n;
        }
        i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
            .then_some((*chosen).min(n - 1))
    })
}

/// Most rows shown at once; the rest scroll.
const ROWS: f32 = 10.0;
const ROW_HEIGHT: f32 = 26.0;

/// The rows, the chosen one lit; the row clicked, if any.
pub fn rows(ui: &mut egui::Ui, p: &Palette, list: &[Suggestion], chosen: usize) -> Option<usize> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("location_suggestions")
        .max_height(ROWS * ROW_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, s) in list.iter().enumerate() {
                let row = menu::suggestion_row(ui, p, s.icon(), &s.label, "", i == chosen);
                if i == chosen
                    && ui
                        .ctx()
                        .input(|inp| inp.key_pressed(egui::Key::ArrowDown) || inp.key_pressed(egui::Key::ArrowUp))
                {
                    row.scroll_to_me(None);
                }
                if row.clicked() {
                    clicked = Some(i);
                }
            }
        });
    clicked
}

/// The rows in a list of their own under `field` (the Legacy bars' location
/// field); the row clicked, if any.
pub fn list_area(ui: &egui::Ui, p: &Palette, id: egui::Id, field: Rect, list: &[Suggestion], chosen: usize) -> Option<usize> {
    egui::Area::new(id)
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(field.left_bottom() + Vec2::new(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(field.width().max(240.0));
                ui.set_max_width(field.width().max(240.0));
                rows(ui, p, list, chosen)
            })
            .inner
        })
        .inner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(names: &[(&str, u128)]) -> Vec<Named> {
        names
            .iter()
            .map(|(name, n)| Named {
                name: (*name).into(),
                lower: name.to_lowercase(),
                asset: Uuid::from_u128(*n),
            })
            .collect()
    }

    fn place(parcel: &str, region: &str, handle: RegionHandle) -> TpEntry {
        TpEntry {
            region: region.into(),
            handle,
            position: Vec3::new(10.0, 20.0, 30.0),
            parcel: parcel.into(),
            at: 0,
        }
    }

    fn labels(list: &[Suggestion]) -> Vec<&str> {
        list.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn dictionary_order_follows_the_legacy_viewer() {
        let mut v = vec!["store 10", "Store 2", "beach", "Beach", "Abc", "abc 1", "Zoo"];
        v.sort_by(|a, b| compare_dict(a, b));
        assert_eq!(v, ["Abc", "abc 1", "Beach", "beach", "Store 2", "store 10", "Zoo"]);
        assert_eq!(compare_dict("Same", "Same"), Ordering::Equal);
        assert_eq!(compare_dict("a02", "a2"), Ordering::Greater, "the longer run of digits is greater");
    }

    #[test]
    fn landmarks_match_whatever_the_case_history_as_typed() {
        let lms = named(&[("Chonk Landing", 1), ("KB Store", 2), ("Beach", 3)]);
        let history = [place("Pier", "Chonk", 7), place("Shop", "kb", 8)];
        let list = suggestions(&lms, &history, "chonk");
        assert_eq!(
            labels(&list),
            ["Chonk Landing"],
            "the history's \"Pier, Chonk\" holds \"Chonk\", not \"chonk\""
        );
        let list = suggestions(&lms, &history, "Chonk");
        assert_eq!(labels(&list), ["Chonk Landing", "Pier, Chonk"]);
        assert_eq!(list[0].target, Target::Landmark(Uuid::from_u128(1)));
        assert_eq!(
            list[1].target,
            Target::Place {
                region: "Chonk".into(),
                handle: 7,
                position: Vec3::new(10.0, 20.0, 30.0)
            }
        );
        assert!(suggestions(&lms, &history, "  ").is_empty());
        assert!(suggestions(&lms, &history, "nowhere").is_empty());
    }

    #[test]
    fn one_row_per_label_landmark_first() {
        let lms = named(&[("Pier, Chonk", 1)]);
        let history = [place("Pier", "Chonk", 7), place("Pier", "Chonk", 7)];
        let list = suggestions(&lms, &history, "Pier");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].target, Target::Landmark(Uuid::from_u128(1)));
    }

    #[test]
    fn the_exact_name_wins_else_the_first_in_order() {
        let lms = named(&[("Chonk Landing", 1), ("chonk", 2), ("A Chonk view", 3)]);
        let list = suggestions(&lms, &[], "CHONK");
        assert_eq!(labels(&list), ["A Chonk view", "chonk", "Chonk Landing"]);
        assert_eq!(pick(&list, "CHONK"), Some(1), "exact name, any case");
        let list = suggestions(&lms, &[], "Land");
        assert_eq!(pick(&list, "Land"), Some(0));
        assert_eq!(pick(&[], "Land"), None, "nothing matches: the text is a SLURL or a region");
    }

    #[test]
    fn a_landmark_beats_the_bare_region_and_a_slurl_matches_nothing() {
        let lms = named(&[("KB", 1)]);
        let list = suggestions(&lms, &[], "kb");
        let chosen = &list[pick(&list, "kb").expect("a row")];
        assert!(matches!(chosen.action(), BarAction::TeleportLandmark(asset) if asset == Uuid::from_u128(1)));
        let slurl = "http://maps.secondlife.com/secondlife/KB/128/128/20";
        assert!(suggestions(&lms, &[], slurl).is_empty());
    }
}
