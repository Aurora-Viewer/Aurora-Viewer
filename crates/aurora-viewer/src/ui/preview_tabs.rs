//! Tab hosts of a bulk Open or Properties: membership, selection and close
//! requests, separate from the editors' drafts, requests and close
//! confirmations. Follows LLInventoryAction::doToSelected
//! (indra/newview/llinventoryfunctions.cpp), LLMultiPreview
//! (indra/newview/llpreview.cpp) and LLMultiItemProperties
//! (indra/newview/llfloatermarketplacelistings.cpp), originally LGPL 2.1.
//!
//! Difference with the Legacy Viewer: it puts every preview of a bulk Open
//! into one LLMultiPreview, and opens no host at all (each preview in its own
//! window) when one selected item refuses `isMultiPreviewAllowed`, which
//! environment settings do. Aurora makes one host per family instead
//! (textures and materials, then notecards, sounds and animations, then
//! scripts), each window keeping the size and tools of its kind, and an
//! environment setting in the selection keeps its own editor while the
//! other items are still grouped.
use crate::theme::Palette;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use uuid::Uuid;

#[derive(Clone)]
pub struct Member {
    pub key: Uuid,
    pub name: String,
    pub dirty: bool,
    pub enabled: bool,
    /// A save, discard or cancel question waits for an answer: the strip is
    /// locked, or a click on another tab would hide the question.
    pub asking: bool,
}

struct Group {
    /// Identity of this batch, so a stale view never changes a newer one.
    id: Uuid,
    kind: &'static str,
    /// Lowest number free among the groups of `kind`: the window id is
    /// stable, so its place and size are remembered and egui keeps one
    /// memory entry per slot instead of one per batch.
    slot: usize,
    /// Not drawn yet: its window opens expanded (`opened`).
    fresh: bool,
    keys: Vec<Uuid>,
    selected: Uuid,
    closing: bool,
    labels: Arc<[Member]>,
}

#[derive(Default)]
pub struct Hosts {
    groups: Vec<Group>,
}

#[derive(Clone)]
pub struct View {
    pub id: Uuid,
    window: String,
    pub member: Uuid,
    pub selected: Uuid,
    pub closing: bool,
    labels: Arc<[Member]>,
    close_tab: bool,
    changed: bool,
}

impl Hosts {
    /// Whether a group was made: fewer than two distinct keys make none,
    /// and those windows keep showing on their own. `kind` names the family
    /// of windows (Properties, documents, scripts) in the window id.
    pub fn group(&mut self, kind: &'static str, keys: Vec<Uuid>) -> bool {
        let mut seen = HashSet::new();
        let keys: Vec<_> = keys.into_iter().filter(|id| seen.insert(*id)).collect();
        if keys.len() < 2 {
            return false;
        }
        for group in &mut self.groups {
            group.keys.retain(|id| !seen.contains(id));
            if !group.keys.contains(&group.selected)
                && let Some(id) = group.keys.first()
            {
                group.selected = *id;
            }
        }
        self.groups.retain(|group| !group.keys.is_empty());
        let slot = self.free_slot(kind);
        self.groups.push(Group {
            id: Uuid::new_v4(),
            kind,
            slot,
            fresh: true,
            selected: keys[0],
            keys,
            closing: false,
            labels: Arc::from([]),
        });
        true
    }
    fn free_slot(&self, kind: &str) -> usize {
        (0..)
            .find(|slot| !self.groups.iter().any(|group| group.kind == kind && group.slot == *slot))
            .expect("a free slot")
    }
    pub fn append(&mut self, other: &mut Self) {
        for mut group in other.groups.drain(..) {
            group.slot = self.free_slot(group.kind);
            self.groups.push(group);
        }
    }
    /// Windows of the groups made since the last call: a slot's window may
    /// have been minimized by an earlier group, the new one opens expanded.
    pub fn opened(&mut self) -> Vec<String> {
        self.groups
            .iter_mut()
            .filter_map(|group| std::mem::take(&mut group.fresh).then(|| window_id(group.kind, group.slot)))
            .collect()
    }
    pub fn update(&mut self, members: impl IntoIterator<Item = Member>) {
        let live: HashMap<_, _> = members.into_iter().map(|member| (member.key, member)).collect();
        for group in &mut self.groups {
            group.keys.retain(|id| live.contains_key(id));
            if !group.keys.contains(&group.selected)
                && let Some(id) = group.keys.first()
            {
                group.selected = *id;
            }
            if !live.get(&group.selected).is_some_and(|member| member.enabled)
                && let Some(id) = group.keys.iter().find(|id| live[id].enabled)
            {
                group.selected = *id;
            }
            // every tab disabled (RLVa: no notecard previews) draws nothing
            // that could carry a close on: a close under way is dropped, or
            // the tabs would close themselves once the restriction lifts
            if !group.keys.iter().any(|id| live[id].enabled) {
                group.closing = false;
            }
            group.labels = group.keys.iter().map(|id| live[id].clone()).collect();
        }
        self.groups.retain(|group| !group.keys.is_empty());
    }
    pub fn focus(&mut self, member: Uuid) -> Option<String> {
        let group = self.groups.iter_mut().find(|group| group.keys.contains(&member))?;
        group.selected = member;
        Some(window_id(group.kind, group.slot))
    }
    /// A member that cannot close (a failed save): its group stops closing
    /// and shows it. False for a window in no group.
    pub fn cancel_member(&mut self, member: Uuid) -> bool {
        let Some(group) = self.groups.iter_mut().find(|group| group.keys.contains(&member)) else {
            return false;
        };
        group.selected = member;
        group.closing = false;
        true
    }
    pub fn view(&self, member: Uuid) -> Option<View> {
        let group = self.groups.iter().find(|group| group.keys.contains(&member))?;
        Some(View {
            id: group.id,
            window: window_id(group.kind, group.slot),
            member,
            selected: group.selected,
            closing: group.closing,
            labels: group.labels.clone(),
            close_tab: false,
            changed: false,
        })
    }
    pub fn report(&mut self, view: &mut View) {
        if std::mem::take(&mut view.changed)
            && let Some(group) = self.groups.iter_mut().find(|group| group.id == view.id)
        {
            group.selected = view.selected;
            group.closing = view.closing;
        }
    }
}

/// A group's row of tabs: one line that scrolls sideways when the window is
/// too narrow for every tab, with the mouse wheel as well as Shift + wheel
/// (egui's `always_scroll_the_only_direction`, off by default).
pub fn tab_strip<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::scroll_area::ScrollAreaOutput<R> {
    ui.style_mut().always_scroll_the_only_direction = true;
    egui::ScrollArea::horizontal().id_salt("preview_tabs").show(ui, add)
}

/// The title of every preview group's window, whatever tab is selected:
/// the tabs carry the names and the unsaved marks (LLMultiPreview's
/// MultiPreviewTitle).
pub fn group_title() -> String {
    aurora_i18n::tr!("resource-preview-tabs-title").to_string()
}

pub fn window_id(kind: &str, slot: usize) -> String {
    format!("{kind}_tab_host_{slot}")
}

impl View {
    pub fn active(&self) -> bool {
        self.member == self.selected
    }
    pub fn window_id(&self) -> String {
        self.window.clone()
    }
    pub fn cancel_close(&mut self) {
        self.selected = self.member;
        self.closing = false;
        self.changed = true;
    }
    /// The tabs cannot change while a close goes on or a question waits:
    /// only the selected tab draws its save, discard or cancel question.
    pub fn locked(&self) -> bool {
        self.closing || self.labels.iter().any(|member| member.asking)
    }
    pub fn header(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let locked = self.locked();
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 34.0).max(0.0);
            ui.allocate_ui(egui::vec2(width, 36.0), |ui| {
                tab_strip(ui, |ui| {
                    ui.horizontal(|ui| {
                        for member in self.labels.iter() {
                            let name = if member.dirty {
                                format!("{} *", member.name)
                            } else {
                                member.name.clone()
                            };
                            let selected = member.key == self.selected;
                            let response =
                                super::widgets::pill_tab(ui, p, "", &name, None, false, selected, member.enabled && (selected || !locked));
                            if response.clicked() && !locked {
                                self.selected = member.key;
                                self.changed = true;
                                response.scroll_to_me(None);
                            }
                        }
                    });
                });
            });
            self.close_tab |= super::widgets::action_button(ui, p, "x", "", super::widgets::ActionKind::Flat, egui::Vec2::splat(20.0))
                .on_hover_text(aurora_i18n::tr!("resource-preview-close-tab"))
                .clicked();
        });
    }
    pub fn closing(&mut self, open: &mut bool) {
        if !*open && !self.closing {
            self.closing = true;
            self.changed = true;
        }
        *open &= !self.closing && !self.close_tab;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const KIND: &str = "test";
    fn member(n: u128) -> Member {
        Member {
            key: Uuid::from_u128(n),
            name: format!("Item {n}"),
            dirty: false,
            enabled: true,
            asking: false,
        }
    }
    #[test]
    fn the_wheel_scrolls_a_tab_strip_too_wide_for_its_window() {
        let ctx = egui::Context::default();
        let strip = |ctx: &egui::Context, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(240.0, 120.0))),
                events,
                ..Default::default()
            };
            let mut offset = 0.0;
            let mut output = ctx.run_ui(input, |ui| {
                ui.allocate_ui(egui::vec2(200.0, 36.0), |ui| {
                    offset = tab_strip(ui, |ui| {
                        ui.horizontal(|ui| {
                            for n in 0..12 {
                                let _ = ui.button(format!("A long tab name {n}"));
                            }
                        });
                    })
                    .state
                    .clamped_offset()
                    .x;
                });
            });
            // the first frame uploads the font atlas: TexturesDelta's drop
            // asserts it was taken (debug assertions on, the CI profile)
            output.textures_delta.clear();
            offset
        };
        assert_eq!(strip(&ctx, vec![egui::Event::PointerMoved(egui::pos2(60.0, 15.0))]), 0.0);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        let mut offset = strip(&ctx, vec![egui::Event::PointerMoved(egui::pos2(60.0, 15.0)), wheel]);
        for _ in 0..30 {
            offset = strip(&ctx, Vec::new());
        }
        assert!(offset > 0.0, "a plain vertical wheel moves the tabs sideways: {offset}");
    }

    #[test]
    fn closing_advances_only_after_an_editor_finishes_and_cancel_keeps_remaining_tabs() {
        let mut hosts = Hosts::default();
        let keys = [1, 2, 3].map(Uuid::from_u128);
        hosts.group(KIND, vec![keys[0], keys[1], keys[0], keys[2]]);
        hosts.update([member(1), member(2), member(3)]);
        let mut view = hosts.view(keys[0]).unwrap();
        let mut open = false;
        view.closing(&mut open);
        hosts.report(&mut view);
        assert!(hosts.view(keys[0]).unwrap().active());
        assert!(!hosts.view(keys[1]).unwrap().active());
        hosts.update([member(2), member(3)]); // first editor acknowledged closure
        let mut second = hosts.view(keys[1]).unwrap();
        assert!(second.active() && second.closing);
        second.cancel_close();
        hosts.report(&mut second);
        assert!(!hosts.view(keys[1]).unwrap().closing);
        assert!(hosts.view(keys[2]).is_some());
    }
    #[test]
    fn one_key_makes_no_group() {
        let mut hosts = Hosts::default();
        let key = Uuid::from_u128(1);
        assert!(!hosts.group(KIND, vec![key, key]));
        assert!(hosts.view(key).is_none());
        assert!(!hosts.cancel_member(key), "a window in no group");
        assert!(hosts.group(KIND, vec![key, Uuid::from_u128(2)]));
        assert!(hosts.cancel_member(key));
    }
    #[test]
    fn a_group_with_every_tab_disabled_drops_a_close_under_way() {
        let mut hosts = Hosts::default();
        let keys = [1, 2].map(Uuid::from_u128);
        hosts.group(KIND, keys.to_vec());
        hosts.update([member(1), member(2)]);
        let mut view = hosts.view(keys[0]).unwrap();
        view.closing(&mut false);
        hosts.report(&mut view);
        assert!(hosts.view(keys[0]).unwrap().closing);
        // RLVa takes the previews away: nothing draws, nothing carries the close
        let disabled = |n| Member {
            enabled: false,
            ..member(n)
        };
        hosts.update([disabled(1), disabled(2)]);
        assert!(!hosts.view(keys[0]).unwrap().closing);
        // the restriction lifts: the tabs stay open
        hosts.update([member(1), member(2)]);
        assert!(!hosts.view(keys[0]).unwrap().closing && hosts.view(keys[1]).is_some());
    }
    #[test]
    fn a_new_group_takes_the_lowest_free_window_of_its_kind() {
        let mut hosts = Hosts::default();
        let keys: Vec<_> = (1..=10).map(Uuid::from_u128).collect();
        let window = |hosts: &Hosts, key: Uuid| hosts.view(key).unwrap().window_id();
        hosts.group(KIND, keys[0..2].to_vec());
        hosts.group(KIND, keys[2..4].to_vec());
        hosts.group("other", keys[4..6].to_vec());
        let first = window(&hosts, keys[0]);
        assert_ne!(first, window(&hosts, keys[2]));
        assert_ne!(first, window(&hosts, keys[4]), "each kind has windows of its own");
        assert_eq!(hosts.opened().len(), 3);
        assert!(hosts.opened().is_empty(), "opened once");
        // the first group closes: the next one opens in its window
        hosts.update((3..=6).map(member));
        hosts.group(KIND, keys[6..8].to_vec());
        assert_eq!(window(&hosts, keys[6]), first);
        assert_eq!(hosts.opened().as_slice(), std::slice::from_ref(&first));
        // a group made in another list takes a window free in this one
        let mut other = Hosts::default();
        other.group(KIND, keys[8..10].to_vec());
        hosts.append(&mut other);
        let taken = [first, window(&hosts, keys[2])];
        assert!(!taken.contains(&window(&hosts, keys[8])));
    }
    #[test]
    fn a_close_or_a_waiting_question_locks_the_tab_strip() {
        let mut hosts = Hosts::default();
        let keys = [1, 2].map(Uuid::from_u128);
        hosts.group(KIND, keys.to_vec());
        hosts.update([member(1), member(2)]);
        assert!(!hosts.view(keys[0]).unwrap().locked());
        // the tab's own close button asked to save or discard
        let asking = Member { asking: true, ..member(1) };
        hosts.update([asking, member(2)]);
        assert!(hosts.view(keys[1]).unwrap().locked());
        hosts.update([member(1), member(2)]);
        let mut view = hosts.view(keys[0]).unwrap();
        view.closing(&mut false);
        hosts.report(&mut view);
        assert!(hosts.view(keys[0]).unwrap().locked(), "the whole group closes");
    }
    #[test]
    fn regrouping_does_not_let_stale_host_events_change_the_new_batch() {
        let mut hosts = Hosts::default();
        let keys = [1, 2, 3].map(Uuid::from_u128);
        hosts.group(KIND, keys[..2].to_vec());
        hosts.update([member(1), member(2)]);
        let mut stale = hosts.view(keys[0]).unwrap();
        stale.cancel_close();
        hosts.group(KIND, vec![keys[0], keys[2]]);
        hosts.update([member(1), member(2), member(3)]);
        hosts.focus(keys[2]);
        hosts.report(&mut stale);
        assert!(hosts.view(keys[2]).unwrap().active());
        assert_ne!(hosts.view(keys[0]).unwrap().id, hosts.view(keys[1]).unwrap().id);
    }
}
