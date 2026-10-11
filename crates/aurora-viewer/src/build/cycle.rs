//! Cursor-depth selection requested for Aurora: Firestorm's
//! LLViewerWindow::handleScrollWheel only zooms the camera. Selection units
//! and wire messages still follow LLToolSelect::handleObjectSelection
//! (lltoolselect.cpp, originally LGPL 2.1).
use super::*;

#[derive(Default)]
pub(super) struct Wheel {
    context: Option<((f32, f32), bool)>,
    remainder: f32,
}

impl Wheel {
    fn steps(&mut self, delta: f32, cursor: (f32, f32), linked: bool) -> i32 {
        if !delta.is_finite() || delta == 0.0 {
            return 0;
        }
        let context = (cursor, linked);
        if self.context != Some(context) || self.remainder * delta < 0.0 {
            self.remainder = 0.0;
        }
        self.context = Some(context);
        self.remainder += delta;
        let steps = self.remainder.trunc() as i32;
        self.remainder -= steps as f32;
        steps
    }
}

impl BuildTool {
    pub fn reset_selection_wheel(&mut self) {
        self.wheel_selection = Wheel::default();
    }

    /// High-resolution wheels accumulate a full notch before changing objects.
    pub fn selection_wheel_steps(&mut self, delta: f32, cursor: (f32, f32), linked: bool) -> i32 {
        self.wheel_selection.steps(delta, cursor, linked)
    }

    /// `hits` are prim indices ordered from front to back, including occluded
    /// surfaces. A linkset appears only once outside individual-part editing.
    pub fn cycle_under_cursor(&mut self, world: &mut World, s: &BuildSettings, hits: &[usize], steps: i32) {
        if steps == 0 {
            return;
        }
        let primary = self.selection.first().and_then(|key| world.objects.index_of(key));
        let root = s
            .edit_linked
            .then(|| primary.or_else(|| hits.first().copied()).map(|idx| root_of(world, idx)))
            .flatten();
        let mut seen = HashSet::new();
        let units: Vec<_> = hits
            .iter()
            .copied()
            .filter(|&idx| root.is_none_or(|root| root_of(world, idx) == root))
            .filter_map(|idx| Self::unit_of(world, idx, s.edit_linked).map(|key| (idx, key)))
            .filter(|(_, key)| seen.insert(*key))
            .collect();
        if units.is_empty() {
            return;
        }
        // Scrolling down goes deeper; scrolling up goes nearer.
        // Starting a new stack selects its nearest surface in either direction.
        let offset = -i64::from(steps);
        let current = self.selection.first().and_then(|key| units.iter().position(|(_, k)| k == key));
        let next = match current {
            Some(i) => i as i64 + offset,
            None => offset - offset.signum(),
        }
        .rem_euclid(units.len() as i64) as usize;
        let (idx, key) = units[next];
        if self.selection.as_slice() != [key] || self.selection_linked != s.edit_linked {
            self.deselect_all(world);
            self.faces.clear();
            self.last_face = None;
            self.click_select(world, s, Some(idx), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;

    fn stack() -> (World, Vec<usize>) {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
            world.apply(ev);
        }
        let hits = (99610..=99614)
            .map(|id| world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap())
            .collect();
        (world, hits)
    }

    fn selected(tool: &BuildTool, world: &World) -> Vec<u32> {
        tool.selection
            .iter()
            .filter_map(|key| world.objects.index_of(key))
            .map(|idx| world.objects.get(idx).unwrap().key.local_id)
            .collect()
    }

    #[test]
    fn wheel_accumulates_notches_and_discards_remainder_on_direction_or_context_change() {
        let mut w = Wheel::default();
        assert_eq!(w.steps(0.4, (1.0, 2.0), false), 0);
        assert_eq!(w.steps(0.7, (1.0, 2.0), false), 1);
        assert_eq!(w.steps(-0.6, (1.0, 2.0), false), 0);
        assert_eq!(w.steps(-0.6, (1.0, 2.0), false), -1);
        assert_eq!(w.steps(-0.9, (2.0, 2.0), false), 0);
        assert_eq!(w.steps(-0.2, (2.0, 2.0), true), 0);
        assert_eq!(w.steps(f32::NAN, (2.0, 2.0), true), 0);
        assert_eq!(w.steps(-2.8, (2.0, 2.0), true), -3);
    }

    #[test]
    fn object_cycle_deduplicates_linksets_reverses_wraps_and_sends_selection_messages() {
        let (mut world, hits) = stack();
        let mut tool = BuildTool::default();
        let s = BuildSettings::default();
        tool.click_select(&mut world, &s, Some(hits[0]), false);
        tool.out.clear();
        tool.cycle_under_cursor(&mut world, &s, &hits, -1);
        assert_eq!(selected(&tool, &world), [99612]);
        assert!(matches!(&tool.out[0], NetCommand::Build(BuildCmd::Deselect { local_ids, .. }) if local_ids == &[99610, 99611]));
        assert!(matches!(&tool.out[1], NetCommand::Build(BuildCmd::Select { local_ids, .. }) if local_ids == &[99612, 99613]));
        tool.cycle_under_cursor(&mut world, &s, &hits, -1);
        assert_eq!(selected(&tool, &world), [99614]);
        tool.cycle_under_cursor(&mut world, &s, &hits, -1);
        assert_eq!(selected(&tool, &world), [99610]);
        tool.cycle_under_cursor(&mut world, &s, &hits, 1);
        assert_eq!(selected(&tool, &world), [99614]);
        tool.cycle_under_cursor(&mut world, &s, &hits, -2);
        assert_eq!(selected(&tool, &world), [99612]);
    }

    #[test]
    fn linked_cycle_stays_in_the_primary_linkset_and_only_visits_hit_prims() {
        let (mut world, hits) = stack();
        let mut tool = BuildTool::default();
        let s = BuildSettings {
            edit_linked: true,
            ..Default::default()
        };
        tool.click_select(&mut world, &s, Some(hits[0]), false);
        tool.cycle_under_cursor(&mut world, &s, &hits, -1);
        assert_eq!(selected(&tool, &world), [99611]);
        tool.cycle_under_cursor(&mut world, &s, &hits, -1);
        assert_eq!(selected(&tool, &world), [99610]);
        tool.cycle_under_cursor(&mut world, &s, &hits, 1);
        assert_eq!(selected(&tool, &world), [99611]);
        tool.out.clear();
        tool.cycle_under_cursor(&mut world, &s, &hits[2..], -1);
        assert_eq!(selected(&tool, &world), [99611]);
        assert!(tool.out.is_empty());
        tool.cycle_under_cursor(&mut world, &s, &hits[0..1], -1);
        assert_eq!(selected(&tool, &world), [99610]);
    }

    #[test]
    fn empty_stack_keeps_selection_and_a_new_stack_starts_at_the_front() {
        let (mut world, hits) = stack();
        let mut tool = BuildTool::default();
        let s = BuildSettings::default();
        tool.cycle_under_cursor(&mut world, &s, &hits, 1);
        assert_eq!(selected(&tool, &world), [99610]);
        tool.cycle_under_cursor(&mut world, &s, &[], -1);
        assert_eq!(selected(&tool, &world), [99610]);
        tool.cycle_under_cursor(&mut world, &s, &hits[2..], -1);
        assert_eq!(selected(&tool, &world), [99612]);
    }

    #[test]
    fn cycling_replaces_multiselection_and_clears_stale_faces() {
        let (mut world, hits) = stack();
        let mut tool = BuildTool::default();
        let s = BuildSettings::default();
        tool.click_select(&mut world, &s, Some(hits[2]), false);
        tool.click_select(&mut world, &s, Some(hits[0]), true);
        tool.faces.insert(tool.selection[0], vec![2]);
        tool.last_face = Some(2);
        tool.cycle_under_cursor(&mut world, &s, &hits, -1);
        assert_eq!(selected(&tool, &world), [99612]);
        assert!(tool.faces.is_empty());
        assert_eq!(tool.last_face, None);
    }
}
