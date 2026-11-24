//! The face buttons 0 to 8 of the build window (the human's request,
//! 2026-11-21): one button per face number, for every selected prim at once.
//! An Aurora control: the Legacy Viewer picks faces in the world
//! (LLToolFace::pickCallback, indra/newview/lltoolface.cpp) and steps through
//! them with ◄ ► (LLToolsSelectNextPartFace), both kept beside the buttons.
//!
//! Faces are picked in Move, Rotate and Stretch too, which keep their
//! handles (2026-11-22): with the buttons, with Ctrl+Shift click on an
//! object, and with Shift or Ctrl click when the build settings say so.
//! The Legacy Viewer picks faces only in "Select face", and its Ctrl+Shift
//! click is a plain click (LLToolSelect::handleObjectSelection extends only
//! on Shift or Ctrl alone).
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

/// What Shift+click or Ctrl+click on an object does with the Edit tool,
/// outside "Select face" (BuildSettings::shift_click / ctrl_click).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModifierClick {
    /// Add the object to the selection or take it out, as the Legacy
    /// Viewer does.
    #[default]
    Object,
    /// Select the clicked face alone, as a click in "Select face" does.
    Face,
}

/// A click on an object that picks the face under it, outside "Select face".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FaceClick {
    /// That face alone, on that object alone (a click in "Select face").
    Select,
    /// Add that face or take it out (a Shift click in "Select face").
    Toggle,
}

impl FaceClick {
    /// The face click the modifiers ask for: Ctrl+Shift always toggles,
    /// Shift or Ctrl alone select when their setting says so.
    pub fn of(s: &BuildSettings, mods: Mods) -> Option<Self> {
        match (mods.shift, mods.ctrl) {
            (true, true) => Some(Self::Toggle),
            (true, false) if s.shift_click == ModifierClick::Face => Some(Self::Select),
            (false, true) if s.ctrl_click == ModifierClick::Face => Some(Self::Select),
            _ => None,
        }
    }
}

impl BuildTool {
    /// Faces can be picked: the Edit tool in any mode but Align, which
    /// works on whole linksets.
    pub fn face_picking(&self) -> bool {
        self.open && self.tool == Tool::Edit && self.edit_mode != EditMode::Align
    }

    /// Faces were picked outside "Select face": they show the face-select
    /// pattern, and the texture edits go to them alone.
    pub fn picks_faces(&self) -> bool {
        self.face_picking() && !self.faces.is_empty()
    }

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
    /// them had it already; the prims without it keep their faces. In
    /// "Select face", each prim becomes a selection unit of its own first, as
    /// a Shift face click does; in the other modes the linksets stay whole,
    /// and keep moving as one.
    pub fn pick_face_button(&mut self, world: &mut World, face: u8, toggle: bool) {
        let index = usize::from(face);
        if !self.face_picking() || index >= FACE_BUTTONS {
            return;
        }
        let state = self.face_buttons(world)[index];
        if state == FaceButton::Absent {
            return;
        }
        if self.face_mode() {
            self.select_prims_individually(world);
        }
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

    /// "All" beside the face buttons (the human's request, 2026-11-22):
    /// every face of every selected prim, picked as the buttons pick them,
    /// so that they show; in "Select face" each prim becomes a selection
    /// unit of its own first, as `pick_face_button` does.
    pub fn select_all_faces(&mut self, world: &mut World) {
        self.set_selected_faces(world, |_| false);
    }

    /// "Invert" beside the face buttons (the human's request, 2026-11-22):
    /// each selected prim takes the faces it did not have, and gives back
    /// those it had. A prim with all of them keeps none and stays selected,
    /// so that a second Invert gives them back.
    pub fn invert_faces(&mut self, world: &mut World) {
        self.set_selected_faces(world, |picked| picked);
    }

    /// Every face of each selected prim for which `drop` (given whether the
    /// face is picked now) is false.
    fn set_selected_faces(&mut self, world: &mut World, drop: impl Fn(bool) -> bool) {
        if !self.face_picking() || self.selection.is_empty() {
            return;
        }
        if self.face_mode() {
            self.select_prims_individually(world);
        }
        for idx in self.sel_prims(world) {
            let Some(key) = world.objects.get(idx).map(|o| o.key) else {
                continue;
            };
            let picked = self.sel_faces(world, idx);
            let faces = (0..self.num_faces(world, idx) as u8)
                .filter(|face| !drop(picked.contains(face)))
                .collect();
            self.faces.insert(key, faces);
        }
    }

    /// ◄ ► beside the face buttons, and Ctrl+, / Ctrl+. but with "Edit
    /// linked" outside "Select face" (`select_next_part`; the human's
    /// choice, 2026-11-23). Shift adds to the selection.
    /// - Faces picked on one prim alone: its faces one by one, then the next
    ///   / previous prim with `edit_linked` (`step_prim_face`, the Legacy
    ///   Viewer's step).
    /// - On several prims: the previous / next face number, picked as its
    ///   button picks it, on every selected prim that has it; the other
    ///   objects stay selected (the human's request, 2026-11-22). With
    ///   every face picked, ► starts at face 0 and ◄ at the last one; past
    ///   the last face, around to 0.
    pub fn select_next_face(&mut self, world: &mut World, edit_linked: bool, next: bool, include: bool) {
        if !self.face_picking() {
            return;
        }
        let picked: Vec<usize> = self
            .sel_prims(world)
            .into_iter()
            .filter(|idx| !self.sel_faces(world, *idx).is_empty())
            .collect();
        if let [idx] = picked[..] {
            self.step_prim_face(world, idx, edit_linked, next, include);
            return;
        }
        let states = self.face_buttons(world);
        // the faces below the last one are there: the prim that has the last has them
        let count = states.iter().take_while(|state| **state != FaceButton::Absent).count();
        if count == 0 {
            return;
        }
        let all = states[..count].iter().all(|state| *state == FaceButton::On);
        let cur = if all && count > 1 {
            None
        } else {
            self.last_face
                .map(usize::from)
                .filter(|face| *face < count)
                .or_else(|| states.iter().position(|state| matches!(state, FaceButton::On | FaceButton::Mixed)))
        };
        let face = match (cur, next) {
            (None, true) => 0,
            (None, false) => count - 1,
            (Some(cur), true) => (cur + 1) % count,
            (Some(cur), false) => (cur + count - 1) % count,
        };
        if !include {
            self.pick_face_button(world, face as u8, false);
        } else if states[face] != FaceButton::On {
            self.pick_face_button(world, face as u8, true);
        }
        self.last_face = Some(face as u8);
    }

    /// A click on face `face` of prim `idx` outside "Select face" (Move,
    /// Rotate, Stretch), as `face_click` does in it, with the linksets kept
    /// whole when "Edit linked" is off: the other prims of the clicked
    /// linkset are selected with no face.
    /// - `Select`: that object alone, with that face alone.
    /// - `Toggle`: an object not selected is added with that face alone;
    ///   on a selected one the face is added or taken out, and the object
    ///   leaves the selection with its last face (LLToolFace).
    pub fn face_pick_click(&mut self, world: &mut World, s: &BuildSettings, idx: usize, face: u8, click: FaceClick) {
        if self.replacement_active() || usize::from(face) >= self.num_faces(world, idx) {
            return;
        }
        if click == FaceClick::Toggle && self.sel_prims(world).contains(&idx) {
            self.toggle_face(world, idx, face);
            self.last_face = Some(face);
            return;
        }
        if click == FaceClick::Select {
            // what click_select checks before a click (RLVa)
            let Some(unit) = Self::unit_of(world, idx, s.edit_linked) else {
                return;
            };
            if !world.rlva_can_edit(idx) {
                return;
            }
            if self.selection != [unit] || self.selection_linked != s.edit_linked {
                self.deselect_all(world);
                self.selection_linked = s.edit_linked;
                self.add(world, unit);
            }
            self.faces.clear();
        } else {
            // added as a Shift click adds it (RLVa, world / worn / HUD spaces)
            self.click_select(world, s, Some(idx), true);
        }
        let Some(unit) = Self::unit_of(world, idx, self.selection_linked).filter(|u| self.is_selected(u)) else {
            return;
        };
        for (prim, _) in highlighted_of(world, &[unit], self.selection_linked) {
            if let Some(key) = world.objects.get(prim).map(|o| o.key) {
                self.faces.insert(key, if prim == idx { vec![face] } else { Vec::new() });
            }
        }
        self.last_face = Some(face);
    }

    /// Face `face` of the selected prim `idx` added or taken out; its
    /// object leaves the selection once none of its prims has a face left.
    fn toggle_face(&mut self, world: &mut World, idx: usize, face: u8) {
        let Some(key) = world.objects.get(idx).map(|o| o.key) else { return };
        let n = self.num_faces(world, idx);
        let list = self.faces.entry(key).or_insert_with(|| (0..n as u8).collect());
        if let Some(pos) = list.iter().position(|f| *f == face) {
            list.remove(pos);
        } else {
            list.push(face);
        }
        let Some(unit) = Self::unit_of(world, idx, self.selection_linked) else {
            return;
        };
        let prims: Vec<_> = highlighted_of(world, &[unit], self.selection_linked)
            .into_iter()
            .filter_map(|(prim, _)| world.objects.get(prim).map(|o| o.key))
            .collect();
        if prims.iter().all(|k| self.faces.get(k).is_some_and(Vec::is_empty)) {
            for k in &prims {
                self.faces.remove(k);
            }
            self.deselect(world, &[unit]);
        }
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
    fn a_whole_linkset_becomes_prims_and_absent_faces_or_align_do_nothing() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        select_all(&mut world, &mut tool, &settings, prims);
        assert!(!tool.selection_linked);
        tool.pick_face_button(&mut world, 8, false);
        assert!(!tool.selection_linked);
        assert!(tool.faces.is_empty());
        tool.pick_face_button(&mut world, 0, false);
        assert!(tool.selection_linked);
        assert_eq!(tool.selection.len(), 3);
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Align);
        assert!(tool.faces.is_empty());
        tool.pick_face_button(&mut world, 1, false);
        assert!(tool.faces.is_empty());
        assert!(tool.face_picks(&world).is_empty());
    }

    #[test]
    fn in_move_the_buttons_pick_faces_and_keep_the_linksets_whole() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        select_all(&mut world, &mut tool, &settings, prims);
        assert!(!tool.picks_faces());
        assert!(tool.face_picks(&world).is_empty());
        tool.pick_face_button(&mut world, 3, false);
        assert_eq!(tool.edit_mode, EditMode::Move);
        assert!(!tool.selection_linked);
        assert_eq!(tool.selection.len(), 2);
        assert_eq!(tool.sel_faces(&world, prims[0]), [3]);
        assert!(tool.sel_faces(&world, prims[1]).is_empty());
        assert_eq!(tool.sel_faces(&world, prims[2]), [3]);
        let picks = tool.face_picks(&world);
        assert_eq!((picks[&prims[0]], picks[&prims[1]], picks[&prims[2]]), (1 << 3, 0, 1 << 3));
        // Rotate and Stretch keep them
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Rotate);
        assert_eq!(tool.sel_te(&world).len(), 2);
        // an object selected again comes back with all its faces
        tool.click_select(&mut world, &settings, Some(prims[0]), true);
        tool.click_select(&mut world, &settings, Some(prims[0]), true);
        assert_eq!(tool.sel_faces(&world, prims[0]).len(), 8);
        assert_eq!(tool.sel_faces(&world, prims[1]).len(), 3);
        assert_eq!(tool.sel_faces(&world, prims[2]), [3]);
        // "Edit linked" keeps the picks of the prims
        tool.set_edit_linked(&mut world, &mut settings, true);
        assert_eq!(tool.sel_faces(&world, prims[2]), [3]);
        tool.set_edit_linked(&mut world, &mut settings, false);
        assert_eq!(tool.sel_faces(&world, prims[2]), [3]);
        // entering "Select face" keeps them too
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
        tool.pick_face_button(&mut world, 0, true);
        assert_eq!(tool.sel_faces(&world, prims[2]), [3, 0]);
        assert_eq!(tool.sel_faces(&world, prims[0]).len(), 8);
    }

    #[test]
    fn all_and_invert_take_every_face_or_the_others_and_keep_the_objects() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        select_all(&mut world, &mut tool, &settings, prims);
        let objects = tool.selection.clone();
        // All picks the faces every object had, so that they show
        tool.select_all_faces(&mut world);
        assert!(tool.picks_faces());
        for idx in prims {
            assert_eq!(tool.sel_faces(&world, idx).len(), tool.num_faces(&world, idx));
        }
        assert_eq!(tool.face_picks(&world)[&prims[1]], 0b111);
        // Invert: none, the objects kept, then all again
        tool.invert_faces(&mut world);
        assert!(prims.iter().all(|idx| tool.sel_faces(&world, *idx).is_empty()));
        assert_eq!(tool.selection, objects);
        tool.invert_faces(&mut world);
        assert_eq!(tool.sel_faces(&world, prims[1]), [0, 1, 2]);
        tool.pick_face_button(&mut world, 1, false);
        tool.invert_faces(&mut world);
        assert_eq!(tool.sel_faces(&world, prims[0]), [0, 2, 3, 4, 5, 6, 7]);
        assert_eq!(tool.sel_faces(&world, prims[1]), [0, 2]);
        assert_eq!((tool.edit_mode, &tool.selection), (EditMode::Move, &objects));
        // in "Select face" each prim becomes a unit; Align does nothing
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
        tool.select_all_faces(&mut world);
        assert_eq!(tool.selection.len(), tool.sel_prims(&world).len());
        assert_eq!(tool.sel_faces(&world, prims[1]), [0, 1, 2]);
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Align);
        tool.invert_faces(&mut world);
        tool.select_all_faces(&mut world);
        assert!(tool.faces.is_empty());
    }

    fn click(world: &mut World, tool: &mut BuildTool, s: &mut BuildSettings, idx: usize, face: i32, mods: Mods) {
        let surface = aurora_net::TouchSurface {
            face,
            ..Default::default()
        };
        tool.mouse_down(world, s, None, Some(idx), Some(surface), (0.0, 0.0), mods);
        tool.mouse_up(world, s, None, Mods::default());
    }

    const SHIFT: Mods = Mods {
        shift: true,
        ctrl: false,
        alt: false,
    };
    const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
        alt: false,
    };
    const BOTH: Mods = Mods {
        shift: true,
        ctrl: true,
        alt: false,
    };

    #[test]
    fn ctrl_shift_click_adds_or_takes_out_the_clicked_face_and_keeps_the_mode() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        tool.click_select(&mut world, &settings, Some(prims[2]), false);
        // an object not selected comes in with that face alone
        click(&mut world, &mut tool, &mut settings, prims[0], 2, BOTH);
        assert_eq!(tool.edit_mode, EditMode::Move);
        assert!(!tool.selection_linked);
        assert_eq!(tool.selection.len(), 2);
        assert_eq!(tool.sel_faces(&world, prims[0]), [2]);
        assert!(tool.sel_faces(&world, prims[1]).is_empty());
        assert_eq!(tool.sel_faces(&world, prims[2]).len(), 4);
        // on a selected prim the face is added or taken out
        click(&mut world, &mut tool, &mut settings, prims[1], 1, BOTH);
        assert_eq!(tool.sel_faces(&world, prims[1]), [1]);
        click(&mut world, &mut tool, &mut settings, prims[2], 0, BOTH);
        assert_eq!(tool.sel_faces(&world, prims[2]), [1, 2, 3]);
        // a face the prim lacks does nothing
        click(&mut world, &mut tool, &mut settings, prims[2], 7, BOTH);
        assert_eq!(tool.sel_faces(&world, prims[2]), [1, 2, 3]);
        // the object leaves with the last face of its prims
        click(&mut world, &mut tool, &mut settings, prims[0], 2, BOTH);
        assert_eq!(tool.selection.len(), 2);
        click(&mut world, &mut tool, &mut settings, prims[1], 1, BOTH);
        assert_eq!(tool.sel_prims(&world), [prims[2]]);
        assert!(!tool.faces.contains_key(&world.objects.get(prims[0]).unwrap().key));
    }

    #[test]
    fn shift_and_ctrl_click_add_the_object_by_default_and_select_the_face_when_set() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        assert_eq!(
            (settings.shift_click, settings.ctrl_click),
            (ModifierClick::Object, ModifierClick::Object)
        );
        assert_eq!(FaceClick::of(&settings, SHIFT), None);
        assert_eq!(FaceClick::of(&settings, CTRL), None);
        assert_eq!(FaceClick::of(&settings, BOTH), Some(FaceClick::Toggle));
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        tool.click_select(&mut world, &settings, Some(prims[2]), false);
        click(&mut world, &mut tool, &mut settings, prims[0], 1, SHIFT);
        assert_eq!(tool.selection.len(), 2);
        assert!(tool.faces.is_empty());
        // "Select the face": the other objects leave, that face alone
        settings.shift_click = ModifierClick::Face;
        click(&mut world, &mut tool, &mut settings, prims[0], 1, SHIFT);
        assert_eq!(tool.selection.len(), 1);
        assert_eq!(tool.sel_prims(&world).len(), 2);
        assert_eq!(tool.sel_faces(&world, prims[0]), [1]);
        assert!(tool.sel_faces(&world, prims[1]).is_empty());
        assert_eq!(tool.edit_mode, EditMode::Move);
        // Ctrl keeps adding the object
        click(&mut world, &mut tool, &mut settings, prims[2], 0, CTRL);
        assert_eq!(tool.selection.len(), 2);
        assert_eq!(tool.sel_faces(&world, prims[2]).len(), 4);
        assert_eq!(tool.sel_faces(&world, prims[0]), [1]);
        settings.ctrl_click = ModifierClick::Face;
        click(&mut world, &mut tool, &mut settings, prims[1], 2, CTRL);
        assert_eq!(tool.selection.len(), 1);
        assert_eq!(tool.sel_faces(&world, prims[1]), [2]);
        assert!(tool.sel_faces(&world, prims[0]).is_empty());
        // with "Edit linked", the prim alone
        settings.edit_linked = true;
        click(&mut world, &mut tool, &mut settings, prims[1], 0, SHIFT);
        assert!(tool.selection_linked);
        assert_eq!(tool.sel_prims(&world), [prims[1]]);
        assert_eq!(tool.sel_faces(&world, prims[1]), [0]);
        // in "Select face", Shift keeps toggling faces
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
        click(&mut world, &mut tool, &mut settings, prims[1], 1, SHIFT);
        assert_eq!(tool.sel_faces(&world, prims[1]), [0, 1]);
    }

    #[test]
    fn the_face_arrows_step_every_selected_prim_and_keep_the_objects() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        select_all(&mut world, &mut tool, &settings, prims);
        let faces = |tool: &BuildTool, world: &World| prims.map(|idx| tool.sel_faces(world, idx));
        tool.select_next_face(&mut world, false, true, false);
        assert_eq!(tool.edit_mode, EditMode::Move);
        assert!(!tool.selection_linked);
        assert_eq!(tool.selection.len(), 2);
        assert_eq!(faces(&tool, &world), [vec![0], vec![0], vec![0]]);
        tool.select_next_face(&mut world, false, true, false);
        assert_eq!(faces(&tool, &world), [vec![1], vec![1], vec![1]]);
        // Shift adds the next one
        tool.select_next_face(&mut world, false, true, true);
        assert_eq!(faces(&tool, &world), [vec![1, 2], vec![1, 2], vec![1, 2]]);
        tool.select_next_face(&mut world, false, false, false);
        assert_eq!(faces(&tool, &world), [vec![1], vec![1], vec![1]]);
        // from every face picked, the last one backwards: the 8-face root's,
        // the others keep none
        tool.faces.clear();
        tool.select_next_face(&mut world, false, false, false);
        assert_eq!(faces(&tool, &world), [vec![7], vec![], vec![]]);
        assert_eq!(tool.selection.len(), 2);
        // in "Select face" too, every prim a unit of its own
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Face);
        tool.faces.clear();
        tool.select_next_face(&mut world, false, true, false);
        assert_eq!(tool.selection.len(), 3);
        assert_eq!(faces(&tool, &world), [vec![0], vec![0], vec![0]]);
        // nothing in Align
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Align);
        tool.select_next_face(&mut world, false, true, false);
        assert!(tool.faces.is_empty());
    }

    #[test]
    fn faces_of_one_prim_step_one_by_one_and_cross_the_links_with_edit_linked() {
        // the human's choice, 2026-11-23: faces picked on one prim alone
        // step as LLToolsSelectNextPartFace steps them, in every edit mode
        let (mut world, mut tool, mut settings, prims) = fixture();
        let [root, child, _] = prims;
        for mode in [EditMode::Move, EditMode::Face] {
            for edit_linked in [false, true] {
                settings.edit_linked = edit_linked;
                tool.set_edit_mode(&mut world, &mut settings, mode);
                tool.deselect_all(&world);
                tool.faces.clear();
                tool.click_select(&mut world, &settings, Some(root), false);
                tool.pick_face_button(&mut world, 6, false);
                assert_eq!(tool.sel_te(&world), [(root, 6)], "{mode:?}, linked {edit_linked}");
                tool.select_next_face(&mut world, edit_linked, true, false);
                assert_eq!(tool.sel_te(&world), [(root, 7)], "{mode:?}, linked {edit_linked}");
                tool.select_next_face(&mut world, edit_linked, true, false);
                let past = if edit_linked { (child, 0) } else { (root, 0) };
                assert_eq!(tool.sel_te(&world), [past], "{mode:?}, linked {edit_linked}");
                tool.select_next_face(&mut world, edit_linked, false, false);
                assert_eq!(tool.sel_te(&world), [(root, 7)], "{mode:?}, linked {edit_linked}");
                // Shift adds the face before
                tool.select_next_face(&mut world, edit_linked, false, true);
                assert_eq!(tool.sel_faces(&world, root), [7, 6], "{mode:?}, linked {edit_linked}");
            }
        }
    }

    #[test]
    fn ctrl_comma_and_period_step_as_the_face_arrows_but_the_links_with_edit_linked() {
        // the human's choice, 2026-11-23: LLToolsSelectNextPartFace's keys
        // follow the face arrows, except "Edit linked" outside "Select
        // face", where they step the links as before
        let (mut world, mut tool, mut settings, prims) = fixture();
        let [root, child, other] = prims;
        let faces = |tool: &BuildTool, world: &World| prims.map(|idx| tool.sel_faces(world, idx));
        // several prims in "Select face": every one of them steps
        select_all(&mut world, &mut tool, &settings, prims);
        tool.select_next_part(&mut world, &mut settings, true, false);
        assert_eq!(faces(&tool, &world), [vec![0], vec![0], vec![0]]);
        tool.select_next_part(&mut world, &mut settings, true, false);
        assert_eq!(faces(&tool, &world), [vec![1], vec![1], vec![1]]);
        // neither "Select face" nor "Edit linked": the face arrows' step,
        // the objects kept whole
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        tool.deselect_all(&world);
        tool.faces.clear();
        select_all(&mut world, &mut tool, &settings, prims);
        tool.select_next_part(&mut world, &mut settings, true, false);
        assert!(!tool.selection_linked);
        assert_eq!(faces(&tool, &world), [vec![0], vec![0], vec![0]]);
        // "Edit linked" outside "Select face": the next link, all its faces
        settings.edit_linked = true;
        tool.deselect_all(&world);
        tool.faces.clear();
        tool.click_select(&mut world, &settings, Some(root), false);
        tool.select_next_part(&mut world, &mut settings, true, false);
        assert_eq!(tool.sel_prims(&world), [child]);
        assert_eq!(tool.sel_faces(&world, child), [0, 1, 2]);
        assert!(!tool.is_selected(&world.objects.get(other).unwrap().key));
    }

    #[test]
    fn once_a_face_is_picked_the_objects_that_kept_all_theirs_show_them() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        tool.click_select(&mut world, &settings, Some(prims[2]), false);
        assert!(tool.face_picks(&world).is_empty());
        click(&mut world, &mut tool, &mut settings, prims[0], 2, BOTH);
        let picks = tool.face_picks(&world);
        assert_eq!(picks[&prims[2]], 0b1111);
        assert_eq!(picks[&prims[0]], 1 << 2);
        assert_eq!(picks[&prims[1]], 0);
        // under another tool, nothing shows
        tool.set_tool(Tool::Create);
        assert!(tool.face_picks(&world).is_empty());
    }

    #[test]
    fn ctrl_right_click_edits_by_default_with_ctrl_alone() {
        let mut s = BuildSettings::default();
        assert_eq!(s.ctrl_right_click, CtrlRightClick::Edit);
        assert!(CtrlRightClick::edits(&s, false, true, false));
        s.ctrl_right_click = CtrlRightClick::Menu;
        assert!(!CtrlRightClick::edits(&s, false, true, false));
        s.ctrl_right_click = CtrlRightClick::Edit;
        for (shift, ctrl, alt) in [
            (false, false, false),
            (true, false, false),
            (true, true, false),
            (false, true, true),
        ] {
            assert!(!CtrlRightClick::edits(&s, shift, ctrl, alt), "{shift} {ctrl} {alt}");
        }
        // settings saved before these options load with the defaults
        let old: BuildSettings = serde_json::from_str("{}").expect("empty build settings");
        assert_eq!(
            (old.shift_click, old.ctrl_click, old.ctrl_right_click, old.double_click_linked),
            (ModifierClick::Object, ModifierClick::Object, CtrlRightClick::Edit, true)
        );
    }

    #[test]
    fn a_double_click_switches_edit_linked_by_default_and_selects_that_prim() {
        let (mut world, mut tool, mut settings, prims) = fixture();
        assert!(BuildSettings::default().double_click_linked, "on by default");
        tool.set_edit_mode(&mut world, &mut settings, EditMode::Move);
        let plain = Mods::default();
        // off: the second press is a click
        settings.double_click_linked = false;
        click(&mut world, &mut tool, &mut settings, prims[1], 0, plain);
        tool.double_press = true;
        click(&mut world, &mut tool, &mut settings, prims[1], 0, plain);
        assert!(!settings.edit_linked);
        assert_eq!(tool.sel_prims(&world).len(), 2);
        settings.double_click_linked = true;
        // on: that prim alone
        tool.double_press = true;
        click(&mut world, &mut tool, &mut settings, prims[1], 0, plain);
        assert!(settings.edit_linked);
        assert!(tool.selection_linked);
        assert_eq!(tool.sel_prims(&world), [prims[1]]);
        // off: its whole linkset
        tool.double_press = true;
        click(&mut world, &mut tool, &mut settings, prims[1], 0, plain);
        assert!(!settings.edit_linked);
        assert_eq!(tool.sel_prims(&world).len(), 2);
        // with Shift or Ctrl, the press is a click
        tool.double_press = true;
        click(&mut world, &mut tool, &mut settings, prims[1], 0, SHIFT);
        assert!(!settings.edit_linked);
        // the flag is read once
        assert!(!tool.double_press);
    }
}
