//! The face buttons 0 to 8 of the build window (the human's request,
//! 2026-11-21): one button per face number, for every selected prim at once.
//! An Aurora control: the Legacy Viewer picks faces in the world
//! (LLToolFace::pickCallback, indra/newview/lltoolface.cpp) and steps through
//! them with ◄ ► (LLToolsSelectNextPartFace), both kept beside the buttons.
use super::*;

/// A prim has at most 9 faces (a hollow, cut box: its outer sides, the
/// hollow, the two ends and the two cut faces, LLVolume::generateFaces); a
/// mesh has at most 8 (LL_SCULPT_MESH_MAX_FACES).
pub const FACE_BUTTONS: usize = 9;

/// What a face button shows for the selected prims.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceButton {
    /// No selected prim has this face: greyed.
    Absent,
    /// Selected on none of the prims that have it.
    Off,
    /// Selected on some of the prims that have it.
    Mixed,
    /// Selected on every prim that has it.
    On,
}

impl BuildTool {
    /// The state of each face button, over every selected prim: a face is
    /// there when at least one of them has it (a mesh of 3 materials with
    /// one of 4: faces 0 to 3).
    pub fn face_buttons(&self, world: &World) -> [FaceButton; FACE_BUTTONS] {
        let prims: Vec<(usize, Vec<u8>)> = self
            .sel_prims(world)
            .into_iter()
            .map(|idx| (self.num_faces(world, idx), self.sel_faces(world, idx)))
            .collect();
        std::array::from_fn(|face| {
            let having: Vec<_> = prims.iter().filter(|(n, _)| face < *n).collect();
            let picked = having.iter().filter(|(_, faces)| faces.contains(&(face as u8))).count();
            match (having.len(), picked) {
                (0, _) => FaceButton::Absent,
                (_, 0) => FaceButton::Off,
                (n, picked) if picked == n => FaceButton::On,
                _ => FaceButton::Mixed,
            }
        })
    }

    /// A click on face button `face` selects that face alone on every
    /// selected prim that has it; a prim without it stays selected with no
    /// face (the human's choice). With `toggle` (Shift), the face is added to
    /// the prims that have it, or taken out of them all when every one of
    /// them had it already; the prims without it keep their faces. Each prim
    /// becomes a selection unit of its own first, as a Shift face click does.
    pub fn pick_face_button(&mut self, world: &mut World, face: u8, toggle: bool) {
        let index = usize::from(face);
        if !self.face_mode() || index >= FACE_BUTTONS {
            return;
        }
        let state = self.face_buttons(world)[index];
        if state == FaceButton::Absent {
            return;
        }
        self.select_prims_individually(world);
        for idx in self.sel_prims(world) {
            let Some(key) = world.objects.get(idx).map(|o| o.key) else {
                continue;
            };
            let n = self.num_faces(world, idx);
            if !toggle {
                self.faces.insert(key, if index < n { vec![face] } else { Vec::new() });
                continue;
            }
            if index >= n {
                continue;
            }
            let list = self.faces.entry(key).or_insert_with(|| (0..n as u8).collect());
            if state == FaceButton::On {
                list.retain(|f| *f != face);
            } else if !list.contains(&face) {
                list.push(face);
            }
        }
        self.last_face = Some(face);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use FaceButton::{Absent, Mixed, Off, On};
    use std::sync::Arc;

    /// The demo's linkset 99610 (with its child 99611) and the object 99614,
    /// in face mode, with 8, 3 and 4 faces.
    fn fixture() -> (World, BuildTool, BuildSettings, [usize; 3]) {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
            world.apply(event);
        }
        let mut tool = BuildTool::default();
        tool.open_build(Tool::Edit);
        let mut settings = BuildSettings::default();
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
        let prims = [99610, 99611, 99614].map(|id| world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap());
        for (idx, n) in prims.into_iter().zip([8, 3, 4]) {
            let id = world.objects.get(idx).unwrap().full_id;
            tool.face_counts.insert(id, n);
        }
        (world, tool, settings, prims)
    }

    fn select_all(world: &mut World, tool: &mut BuildTool, settings: &BuildSettings, prims: [usize; 3]) {
        tool.click_select(world, settings, Some(prims[0]), false);
        tool.click_select(world, settings, Some(prims[2]), true);
    }

    #[test]
    fn a_face_is_there_when_one_selected_prim_has_it() {
        let (mut world, mut tool, settings, prims) = fixture();
        assert_eq!(tool.face_buttons(&world), [Absent; FACE_BUTTONS]);
        tool.click_select(&mut world, &settings, Some(prims[2]), false);
        assert_eq!(tool.face_buttons(&world), [On, On, On, On, Absent, Absent, Absent, Absent, Absent]);
        select_all(&mut world, &mut tool, &settings, prims);
        assert_eq!(tool.face_buttons(&world), [On, On, On, On, On, On, On, On, Absent]);
    }

    #[test]
    fn a_click_selects_that_face_alone_and_keeps_the_prims_without_it_with_none() {
        let (mut world, mut tool, settings, prims) = fixture();
        select_all(&mut world, &mut tool, &settings, prims);
        tool.pick_face_button(&mut world, 3, false);
        assert_eq!(tool.sel_prims(&world).len(), 3);
        assert_eq!(tool.sel_faces(&world, prims[0]), [3]);
        assert!(tool.sel_faces(&world, prims[1]).is_empty());
        assert_eq!(tool.sel_faces(&world, prims[2]), [3]);
        let mut te = tool.sel_te(&world);
        te.sort_unstable();
        let mut expected = vec![(prims[0], 3), (prims[2], 3)];
        expected.sort_unstable();
        assert_eq!(te, expected);
        assert_eq!(tool.face_buttons(&world), [Off, Off, Off, On, Off, Off, Off, Off, Absent]);
        assert_eq!(tool.last_face, Some(3));
        // a face of the 8-face root only
        tool.pick_face_button(&mut world, 6, false);
        assert_eq!(tool.sel_faces(&world, prims[0]), [6]);
        assert!(tool.sel_faces(&world, prims[1]).is_empty());
        assert!(tool.sel_faces(&world, prims[2]).is_empty());
        assert_eq!(tool.sel_te(&world), [(prims[0], 6)]);
    }

    #[test]
    fn shift_adds_the_face_where_it_is_missing_and_takes_it_out_once_everywhere() {
        let (mut world, mut tool, settings, prims) = fixture();
        select_all(&mut world, &mut tool, &settings, prims);
        tool.pick_face_button(&mut world, 1, false);
        tool.pick_face_button(&mut world, 2, true);
        assert_eq!(tool.sel_faces(&world, prims[0]), [1, 2]);
        assert_eq!(tool.sel_faces(&world, prims[1]), [1, 2]);
        assert_eq!(tool.face_buttons(&world)[..4], [Off, On, On, Off]);
        // a Shift click on a face the prims without it lack leaves them alone
        tool.pick_face_button(&mut world, 5, false);
        tool.pick_face_button(&mut world, 3, true);
        assert_eq!(tool.sel_faces(&world, prims[0]), [5, 3]);
        assert!(tool.sel_faces(&world, prims[1]).is_empty());
        assert_eq!(tool.sel_faces(&world, prims[2]), [3]);
        // mixed: the first Shift click completes it, the next takes it out
        tool.pick_face_button(&mut world, 1, false);
        tool.faces.insert(world.objects.get(prims[2]).unwrap().key, vec![0]);
        assert_eq!(tool.face_buttons(&world)[..2], [Mixed, Mixed]);
        tool.pick_face_button(&mut world, 1, true);
        assert_eq!(tool.face_buttons(&world)[..2], [Mixed, On]);
        assert_eq!(tool.sel_faces(&world, prims[2]), [0, 1]);
        tool.pick_face_button(&mut world, 1, true);
        assert_eq!(tool.face_buttons(&world)[..2], [Mixed, Off]);
        assert!(tool.sel_faces(&world, prims[0]).is_empty());
        assert_eq!(tool.sel_faces(&world, prims[2]), [0]);
    }

    #[test]
    fn taking_out_the_last_face_keeps_the_prim_selected_with_none() {
        let (mut world, mut tool, settings, prims) = fixture();
        tool.click_select(&mut world, &settings, Some(prims[2]), false);
        tool.pick_face_button(&mut world, 0, false);
        tool.pick_face_button(&mut world, 0, true);
        assert_eq!(tool.sel_prims(&world), [prims[2]]);
        assert!(tool.sel_faces(&world, prims[2]).is_empty());
        assert!(tool.sel_te(&world).is_empty());
        assert_eq!(tool.face_buttons(&world)[..5], [Off, Off, Off, Off, Absent]);
        assert_eq!(tool.selected_face_numbers(&world), "—");
    }

    #[test]
    fn a_whole_linkset_becomes_prims_and_absent_faces_or_other_modes_do_nothing() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        select_all(&mut world, &mut tool, &settings, prims);
        assert!(!tool.selection_linked);
        tool.pick_face_button(&mut world, 8, false);
        assert!(!tool.selection_linked);
        assert!(tool.faces.is_empty());
        tool.pick_face_button(&mut world, 0, false);
        assert!(tool.selection_linked);
        assert_eq!(tool.selection.len(), 3);
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        let faces = tool.faces.clone();
        tool.pick_face_button(&mut world, 1, false);
        assert_eq!(tool.faces, faces);
    }
}
