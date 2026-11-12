//! LLInventoryAction::doToSelected / LLMultiPreview host membership, separate
//! from the editors' drafts, requests and close confirmations (Legacy Viewer).
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
}

struct Group {
    id: Uuid,
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
    pub member: Uuid,
    pub selected: Uuid,
    pub closing: bool,
    labels: Arc<[Member]>,
    close_tab: bool,
    changed: bool,
}

impl Hosts {
    /// Whether a group was made: fewer than two distinct keys make none,
    /// and those windows keep showing on their own.
    pub fn group(&mut self, keys: Vec<Uuid>) -> bool {
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
        self.groups.push(Group {
            id: Uuid::new_v4(),
            selected: keys[0],
            keys,
            closing: false,
            labels: Arc::from([]),
        });
        true
    }
    pub fn append(&mut self, other: &mut Self) {
        self.groups.append(&mut other.groups);
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
        Some(window_id(group.id))
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

pub fn window_id(id: Uuid) -> String {
    format!("inventory_tab_host_{id}")
}

impl View {
    pub fn active(&self) -> bool {
        self.member == self.selected
    }
    pub fn window_id(&self) -> String {
        window_id(self.id)
    }
    pub fn cancel_close(&mut self) {
        self.selected = self.member;
        self.closing = false;
        self.changed = true;
    }
    pub fn header(&mut self, ui: &mut egui::Ui, p: &Palette) {
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
                            let response =
                                super::widgets::pill_tab(ui, p, "", &name, None, false, member.key == self.selected, member.enabled);
                            if response.clicked() {
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
    fn member(n: u128) -> Member {
        Member {
            key: Uuid::from_u128(n),
            name: format!("Item {n}"),
            dirty: false,
            enabled: true,
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
            let _ = ctx.run_ui(input, |ui| {
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
        hosts.group(vec![keys[0], keys[1], keys[0], keys[2]]);
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
        assert!(!hosts.group(vec![key, key]));
        assert!(hosts.view(key).is_none());
        assert!(!hosts.cancel_member(key), "a window in no group");
        assert!(hosts.group(vec![key, Uuid::from_u128(2)]));
        assert!(hosts.cancel_member(key));
    }
    #[test]
    fn a_group_with_every_tab_disabled_drops_a_close_under_way() {
        let mut hosts = Hosts::default();
        let keys = [1, 2].map(Uuid::from_u128);
        hosts.group(keys.to_vec());
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
    fn regrouping_does_not_let_stale_host_events_change_the_new_batch() {
        let mut hosts = Hosts::default();
        let keys = [1, 2, 3].map(Uuid::from_u128);
        hosts.group(keys[..2].to_vec());
        hosts.update([member(1), member(2)]);
        let mut stale = hosts.view(keys[0]).unwrap();
        stale.cancel_close();
        hosts.group(vec![keys[0], keys[2]]);
        hosts.update([member(1), member(2), member(3)]);
        hosts.focus(keys[2]);
        hosts.report(&mut stale);
        assert!(hosts.view(keys[2]).unwrap().active());
        assert_ne!(hosts.view(keys[0]).unwrap().id, hosts.view(keys[1]).unwrap().id);
    }
}
