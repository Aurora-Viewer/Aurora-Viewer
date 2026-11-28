//! One undo history for the shape draft, shared by the shape editor's
//! sliders and the Avatar Studio shape handles (#5022): a slider drag, a
//! handle drag, a click on a comparison image, the sex choice or an XML
//! import is one step. Aurora's own: the Legacy Viewer's shape editor has
//! Revert only (LLPanelEditWearable::revertChanges).

use std::collections::BTreeMap;

pub type Sliders = BTreeMap<i32, f32>;

/// Steps kept: older ones are dropped.
const LIMIT: usize = 100;

#[derive(Debug, Clone, Default)]
pub struct History {
    undo: Vec<Sliders>,
    redo: Vec<Sliders>,
    /// The values from before the gesture under way.
    open: Option<Sliders>,
}

impl History {
    /// An edit starts or goes on: keep the values from before the gesture.
    pub fn begin(&mut self, before: &Sliders) {
        if self.open.is_none() {
            self.open = Some(before.clone());
        }
    }

    /// The gesture ended (the slider or the handle is released): one step,
    /// unless it came back to where it started.
    pub fn settle(&mut self, now: &Sliders) {
        if let Some(before) = self.open.take()
            && before != *now
        {
            self.undo.push(before);
            if self.undo.len() > LIMIT {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() || self.open.is_some()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() && self.open.is_none()
    }

    /// Back one step; a gesture under way ends first. Whether `now` changed.
    pub fn undo(&mut self, now: &mut Sliders) -> bool {
        self.settle(now);
        let Some(before) = self.undo.pop() else {
            return false;
        };
        self.redo.push(std::mem::replace(now, before));
        true
    }

    pub fn redo(&mut self, now: &mut Sliders) -> bool {
        self.settle(now);
        let Some(after) = self.redo.pop() else {
            return false;
        };
        self.undo.push(std::mem::replace(now, after));
        true
    }
}

/// Ctrl+Z, Ctrl+Shift+Z and Ctrl+Y (Cmd on macOS), as the animation
/// editor's timeline: Some(false) undoes, Some(true) redoes.
pub fn shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<bool> {
    if !(modifiers.ctrl || modifiers.command) || modifiers.alt {
        return None;
    }
    match (key, modifiers.shift) {
        (egui::Key::Z, false) => Some(false),
        (egui::Key::Z, true) | (egui::Key::Y, false) => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(height: f32) -> Sliders {
        BTreeMap::from([(33, height), (80, 0.0)])
    }

    #[test]
    fn a_gesture_is_one_step_and_undo_redo_walk_it_back_and_forth() {
        let mut history = History::default();
        let mut now = values(0.0);
        assert!(!history.can_undo() && !history.can_redo());
        // a drag through many values is one step
        for step in 1..=10 {
            history.begin(&now);
            now = values(step as f32 * 0.1);
        }
        history.settle(&now);
        history.begin(&now);
        now = values(2.0);
        history.settle(&now);
        assert!(history.undo(&mut now));
        assert_eq!(now, values(1.0));
        assert!(history.undo(&mut now));
        assert_eq!(now, values(0.0));
        assert!(!history.undo(&mut now));
        assert!(history.redo(&mut now));
        assert_eq!(now, values(1.0));
        // a new edit drops what could be redone
        history.begin(&now);
        now = values(-1.0);
        history.settle(&now);
        assert!(!history.can_redo());
        assert!(!history.redo(&mut now));
        assert_eq!(now, values(-1.0));
    }

    #[test]
    fn a_gesture_back_to_its_start_is_no_step_and_undo_ends_one_under_way() {
        let mut history = History::default();
        let mut now = values(0.0);
        history.begin(&now);
        // dragged away, then back where it started before the release
        now = values(0.5);
        history.begin(&now);
        now = values(0.0);
        history.settle(&now);
        assert!(!history.can_undo());
        history.begin(&now);
        now = values(0.7);
        // Ctrl+Z during a drag ends it and goes back to before it
        assert!(history.can_undo() && !history.can_redo());
        assert!(history.undo(&mut now));
        assert_eq!(now, values(0.0));
    }

    #[test]
    fn the_history_keeps_its_last_hundred_steps() {
        let mut history = History::default();
        let mut now = values(0.0);
        for step in 1..=150 {
            history.begin(&now);
            now = values(step as f32);
            history.settle(&now);
        }
        let mut undone = 0;
        while history.undo(&mut now) {
            undone += 1;
        }
        assert_eq!(undone, LIMIT);
        assert_eq!(now, values(50.0));
    }

    #[test]
    fn the_shortcuts_follow_the_timeline() {
        let ctrl = egui::Modifiers::CTRL;
        assert_eq!(shortcut(egui::Key::Z, ctrl), Some(false));
        assert_eq!(shortcut(egui::Key::Z, ctrl | egui::Modifiers::SHIFT), Some(true));
        assert_eq!(shortcut(egui::Key::Y, ctrl), Some(true));
        assert_eq!(shortcut(egui::Key::Z, egui::Modifiers::NONE), None);
        assert_eq!(shortcut(egui::Key::Z, ctrl | egui::Modifiers::ALT), None);
    }
}
