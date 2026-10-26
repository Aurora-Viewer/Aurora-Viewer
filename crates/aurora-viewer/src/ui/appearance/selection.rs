//! Outfit selection in display order, shared by the list and gallery.

use std::collections::HashSet;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct Selection {
    pub focus: Option<Uuid>,
    ids: HashSet<Uuid>,
    anchor: Option<Uuid>,
}

impl Selection {
    pub fn contains(&self, id: Uuid) -> bool {
        self.ids.contains(&id)
    }

    pub fn single(&mut self, id: Uuid) {
        self.ids.clear();
        self.ids.insert(id);
        self.focus = Some(id);
        self.anchor = Some(id);
    }

    pub fn ordered(&self, order: &[Uuid]) -> Vec<Uuid> {
        order.iter().filter(|id| self.ids.contains(id)).copied().collect()
    }

    pub fn retain(&mut self, order: &[Uuid]) {
        let visible: HashSet<_> = order.iter().copied().collect();
        self.ids.retain(|id| visible.contains(id));
        if self.focus.is_some_and(|id| !self.ids.contains(&id)) {
            self.focus = order.iter().find(|id| self.ids.contains(id)).copied();
        }
        if self.anchor.is_some_and(|id| !visible.contains(&id)) {
            self.anchor = self.focus;
        }
    }

    pub fn click(&mut self, id: Uuid, order: &[Uuid], modifiers: egui::Modifiers, secondary: bool) {
        if secondary {
            if !self.contains(id) {
                self.single(id);
            }
            return;
        }
        let toggle = modifiers.ctrl || modifiers.command;
        if modifiers.shift
            && let Some(start) = self.anchor.and_then(|anchor| order.iter().position(|id| *id == anchor))
            && let Some(end) = order.iter().position(|candidate| *candidate == id)
        {
            if !toggle {
                self.ids.clear();
            }
            self.ids.extend(order[start.min(end)..=start.max(end)].iter().copied());
            self.focus = Some(id);
        } else if toggle {
            if !self.ids.remove(&id) {
                self.ids.insert(id);
            }
            self.focus = if self.ids.contains(&id) {
                Some(id)
            } else {
                order.iter().find(|id| self.ids.contains(id)).copied()
            };
            self.anchor = Some(id);
        } else {
            self.single(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn order() -> Vec<Uuid> {
        (1..=6).map(Uuid::from_u128).collect()
    }

    #[test]
    fn outfit_selection_toggles_ranges_and_keeps_the_anchor() {
        let order = order();
        let mut s = Selection::default();
        s.click(order[1], &order, egui::Modifiers::NONE, false);
        s.click(order[3], &order, egui::Modifiers::CTRL, false);
        assert_eq!(s.ordered(&order), vec![order[1], order[3]]);
        s.click(order[5], &order, egui::Modifiers::SHIFT, false);
        assert_eq!(s.ordered(&order), order[3..=5]);
        s.click(order[2], &order, egui::Modifiers::SHIFT, false);
        assert_eq!(s.ordered(&order), order[2..=3]);
        s.click(order[2], &order, egui::Modifiers::CTRL, false);
        s.click(order[3], &order, egui::Modifiers::CTRL, false);
        assert!(s.ordered(&order).is_empty());
        assert_eq!(s.focus, None);
    }

    #[test]
    fn outfit_selection_uses_current_order_and_preserves_right_click_selection() {
        let mut order = order();
        let mut s = Selection::default();
        s.single(order[4]);
        order.reverse();
        s.click(order[3], &order, egui::Modifiers::SHIFT, false);
        let selected = order[1..=3].to_vec();
        assert_eq!(s.ordered(&order), selected);
        s.click(selected[0], &order, egui::Modifiers::NONE, true);
        assert_eq!(s.ordered(&order), selected);
        s.click(order[0], &order, egui::Modifiers::NONE, true);
        assert_eq!(s.ordered(&order), vec![order[0]]);
    }

    #[test]
    fn outfit_selection_drops_filtered_deleted_ids_and_supports_additive_ranges() {
        let order = order();
        let mut s = Selection::default();
        s.single(order[0]);
        s.click(order[3], &order, egui::Modifiers::COMMAND, false);
        s.click(order[5], &order, egui::Modifiers::CTRL | egui::Modifiers::SHIFT, false);
        assert_eq!(s.ordered(&order), vec![order[0], order[3], order[4], order[5]]);
        s.retain(&order[1..4]);
        assert_eq!(s.ordered(&order), vec![order[3]]);
        s.click(order[1], &order[1..4], egui::Modifiers::SHIFT, false);
        assert_eq!(s.ordered(&order), order[1..4]);
        s.retain(&[]);
        assert!(s.ordered(&order).is_empty());
        assert_eq!(s.focus, None);
    }
}
