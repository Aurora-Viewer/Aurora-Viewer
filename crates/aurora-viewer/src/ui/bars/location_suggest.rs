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
    let mut labels = std::collections::HashSet::new();
    let mut push = |label: String, target: Target| {
        if labels.insert(label.clone()) {
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

/// What a location field remembers between frames: the list for its text,
/// built again only when the text, the landmarks or the history change
/// (not every frame), and the row chosen. A row chosen with the arrows is
/// kept by where it goes, so a list refreshed under it (landmarks still
/// arriving) cannot turn Enter to another place; otherwise the choice is
/// `pick`'s, again for each new list.
#[derive(Clone, Default)]
pub struct Picker {
    key: Option<(String, usize, u64)>,
    list: std::sync::Arc<[Suggestion]>,
    chosen: usize,
    /// The row the arrows chose, for this text.
    explicit: Option<Target>,
}

impl Picker {
    pub fn load(ctx: &egui::Context, id: egui::Id) -> Picker {
        ctx.data(|d| d.get_temp::<Picker>(id)).unwrap_or_default()
    }

    pub fn store(self, ctx: &egui::Context, id: egui::Id) {
        ctx.data_mut(|d| d.insert_temp(id, self));
    }

    /// The list for `text` (empty when `enabled` is false: the location
    /// hidden, the SLURL the field opens with).
    pub fn refresh(&mut self, landmarks: &std::sync::Arc<[Named]>, history: &[TpEntry], text: &str, enabled: bool) {
        let key = (
            if enabled { text.to_owned() } else { String::new() },
            std::sync::Arc::as_ptr(landmarks).cast::<u8>() as usize,
            history_print(history),
        );
        if self.key.as_ref() == Some(&key) {
            return;
        }
        if self.key.as_ref().is_none_or(|old| old.0 != key.0) {
            // new text: the arrows' choice was for the old one
            self.explicit = None;
        }
        self.list = suggestions(landmarks, history, &key.0).into();
        self.chosen = self
            .explicit
            .as_ref()
            .and_then(|t| self.list.iter().position(|s| &s.target == t))
            .or_else(|| pick(&self.list, &key.0))
            .unwrap_or(0);
        self.key = Some(key);
    }

    pub fn list(&self) -> &[Suggestion] {
        &self.list
    }

    pub fn chosen(&self) -> usize {
        self.chosen
    }

    /// The arrows move the choice, Enter takes it, while the list has rows
    /// and `field` the keyboard. Read before the field, which would lose its
    /// focus on Enter; the field keeps the keyboard and egui does not move
    /// it on (as the chat's suggestions, `chat/suggest.rs`). Returns the
    /// row Enter took, and whether the arrows moved the choice.
    pub fn keys(&mut self, ui: &egui::Ui, field: egui::Id) -> (Option<Suggestion>, bool) {
        let n = self.list.len();
        if n == 0 {
            return (None, false);
        }
        let none = egui::Modifiers::NONE;
        let (down, up, enter) = ui.input_mut(|i| {
            (
                i.consume_key(none, egui::Key::ArrowDown),
                i.consume_key(none, egui::Key::ArrowUp),
                i.consume_key(none, egui::Key::Enter),
            )
        });
        if down {
            self.chosen = (self.chosen + 1) % n;
        }
        if up {
            self.chosen = (self.chosen + n - 1) % n;
        }
        let moved = down || up;
        if moved {
            self.explicit = self.list.get(self.chosen).map(|s| s.target.clone());
        }
        if moved || enter {
            ui.ctx().memory_mut(|m| {
                m.request_focus(field);
                m.move_focus(egui::FocusDirection::None);
            });
        }
        (enter.then(|| self.list[self.chosen.min(n - 1)].clone()), moved)
    }
}

/// What tells two teleport histories apart, cheaply (a hundred places at
/// most): a parcel's name arriving changes a title.
fn history_print(history: &[TpEntry]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for e in history {
        e.region.hash(&mut h);
        e.parcel.hash(&mut h);
        e.handle.hash(&mut h);
        e.position.to_array().map(f32::to_bits).hash(&mut h);
    }
    h.finish()
}

/// Most rows shown at once; the rest scroll.
const ROWS: f32 = 10.0;
const ROW_HEIGHT: f32 = 26.0;

/// The rows, the chosen one lit and scrolled into view when the arrows
/// `moved` it; the row clicked, if any.
pub fn rows(ui: &mut egui::Ui, p: &Palette, list: &[Suggestion], chosen: usize, moved: bool) -> Option<usize> {
    let mut clicked = None;
    egui::ScrollArea::vertical()
        .id_salt("location_suggestions")
        .max_height(ROWS * ROW_HEIGHT)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, s) in list.iter().enumerate() {
                let row = menu::suggestion_row(ui, p, s.icon(), &s.label, "", i == chosen);
                if i == chosen && moved {
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
pub fn list_area(ui: &egui::Ui, p: &Palette, id: egui::Id, field: Rect, list: &[Suggestion], chosen: usize, moved: bool) -> Option<usize> {
    egui::Area::new(id)
        .fade_in(false)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(field.left_bottom() + Vec2::new(0.0, 4.0))
        .show(ui.ctx(), |ui| {
            crate::ui::widgets::overlay(ui, p, egui::Margin::same(4), |ui| {
                ui.set_min_width(field.width().max(240.0));
                ui.set_max_width(field.width().max(240.0));
                rows(ui, p, list, chosen, moved)
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
    fn landmarks_arriving_late_refresh_the_default_choice() {
        let mut picker = Picker::default();
        let early: std::sync::Arc<[Named]> = named(&[("A KB view", 1)]).into();
        picker.refresh(&early, &[], "KB", true);
        assert_eq!(picker.list()[picker.chosen()].label, "A KB view");
        // the exact landmark arrives: Enter now takes it, not row 0
        let late: std::sync::Arc<[Named]> = named(&[("A KB view", 1), ("KB", 2)]).into();
        picker.refresh(&late, &[], "KB", true);
        assert_eq!(picker.list()[picker.chosen()].target, Target::Landmark(Uuid::from_u128(2)));
    }

    #[test]
    fn a_choice_made_with_the_arrows_follows_its_row() {
        let mut picker = Picker::default();
        let early: std::sync::Arc<[Named]> = named(&[("KB", 2), ("KB east", 3)]).into();
        picker.refresh(&early, &[], "KB", true);
        // as the down arrow does
        picker.chosen = 1;
        picker.explicit = Some(Target::Landmark(Uuid::from_u128(3)));
        let late: std::sync::Arc<[Named]> = named(&[("A KB view", 1), ("KB", 2), ("KB east", 3)]).into();
        picker.refresh(&late, &[], "KB", true);
        assert_eq!(picker.list()[picker.chosen()].target, Target::Landmark(Uuid::from_u128(3)));
        // new text: the arrows' choice is forgotten
        picker.refresh(&late, &[], "KB e", true);
        assert!(picker.explicit.is_none());
        picker.refresh(&late, &[], "KB", false);
        assert!(picker.list().is_empty(), "disabled: no list");
    }

    #[test]
    fn many_matches_stay_one_row_per_label() {
        let names: Vec<(String, u128)> = (0..3000).map(|n| (format!("Spot {}", n % 2000), n as u128)).collect();
        let lms: Vec<Named> = names
            .iter()
            .map(|(name, n)| Named {
                name: name.clone(),
                lower: name.to_lowercase(),
                asset: Uuid::from_u128(*n),
            })
            .collect();
        assert_eq!(suggestions(&lms, &[], "spot").len(), 2000);
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
