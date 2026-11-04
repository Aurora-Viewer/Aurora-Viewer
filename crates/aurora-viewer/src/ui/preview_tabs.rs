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
    pub fn group(&mut self, keys: Vec<Uuid>) {
        let mut seen = HashSet::new();
        let keys: Vec<_> = keys.into_iter().filter(|id| seen.insert(*id)).collect();
        if keys.len() < 2 {
            return;
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
            group.labels = group.keys.iter().map(|id| live[id].clone()).collect();
        }
        self.groups.retain(|group| !group.keys.is_empty());
    }
    pub fn focus(&mut self, member: Uuid) -> Option<String> {
        let group = self.groups.iter_mut().find(|group| group.keys.contains(&member))?;
        group.selected = member;
        Some(window_id(group.id))
    }
    pub fn cancel_member(&mut self, member: Uuid) {
        if let Some(group) = self.groups.iter_mut().find(|group| group.keys.contains(&member)) {
            group.selected = member;
            group.closing = false;
        }
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
                egui::ScrollArea::horizontal().id_salt("preview_tabs").show(ui, |ui| {
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
