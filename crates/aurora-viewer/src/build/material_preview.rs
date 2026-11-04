//! Selected-object preview from FSPanelFace::onMatTabChange and
//! LLSelectMgr::hideGLTFMaterial/showGLTFMaterial (indra/newview). Keep the
//! authoritative materials intact: only the effective local render mode changes.
//! User-requested lifetime: retain each object's last mode until region exit or
//! logout, including after deselection and closing the inspector.

use super::{BuildSettings, BuildTool, Tool};
use crate::world::World;
use std::collections::HashSet;

impl BuildTool {
    pub(super) fn update_material_preview(&mut self, world: &mut World, settings: &BuildSettings) {
        let active = self.open
            && (self.tool == Tool::Edit || (settings.legacy_window && self.tool != Tool::Land))
            && settings.expanded
            && self.tab == 3
            && !self.replacement_active();
        if !active {
            self.material_preview_selection.clear();
            self.material_preview_mode = None;
            return;
        }
        let selected: HashSet<_> = self
            .sel_prims(world)
            .into_iter()
            .filter(|&idx| self.can_modify(world, idx))
            .filter_map(|idx| world.objects.get(idx).map(|o| o.full_id))
            .collect();
        let preview = self.ui.tex_tab == 0;
        // Apply only a new inspection choice. In particular, a region change
        // clears the world's overrides without a still-open inspector applying
        // its unchanged selection again on the next frame.
        if self.material_preview_mode == Some(preview) && self.material_preview_selection == selected {
            return;
        }
        for id in &selected {
            if let Some(idx) = world.objects.index_of_uuid(id) {
                world.objects.set_blinn_phong_preview(idx, preview);
            }
        }
        self.material_preview_selection = selected;
        self.material_preview_mode = Some(preview);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::{EditMode, Mods};
    use std::sync::Arc;
    use uuid::Uuid;

    fn fixture() -> (World, BuildTool, BuildSettings, [usize; 3]) {
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
            world.apply(event);
        }
        let objects = [99610, 99611, 99612].map(|id| world.objects.index_of_uuid(&crate::demo::action_id(id)).unwrap());
        for idx in objects {
            let o = world.objects.get_mut(idx).unwrap();
            o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
            o.extra.render_materials = (0..6).map(|face| (face, Uuid::from_u128(0xabc))).collect();
            world.gltf_overrides.insert(
                o.key,
                vec![(
                    0,
                    aurora_assets::material::PbrOverride {
                        metallic_factor: Some(0.4),
                        ..Default::default()
                    },
                )]
                .into(),
            );
        }
        let settings = BuildSettings::default();
        let mut tool = BuildTool::default();
        tool.open_build(Tool::Edit);
        tool.tab = 3;
        tool.click_select(&mut world, &settings, Some(objects[0]), false);
        tool.out.clear();
        (world, tool, settings, objects)
    }

    fn tick(tool: &mut BuildTool, world: &mut World, settings: &BuildSettings) {
        tool.update(world, settings, None, (0.0, 0.0), true, Mods::default());
    }

    #[test]
    fn legacy_material_tabs_keep_previewing_when_another_tool_shares_the_tabs() {
        let (mut world, mut tool, mut settings, [root, _, _]) = fixture();
        settings.legacy_window = true;
        for mode in [Tool::Edit, Tool::Focus, Tool::Grab, Tool::Create] {
            tool.set_tool(mode);
            for tab in [0, 1] {
                tool.ui.tex_tab = tab;
                tool.update_material_preview(&mut world, &settings);
                assert_eq!(world.objects.get(root).unwrap().visual_material(0).is_none(), tab == 0);
            }
        }
        assert!(tool.out.is_empty(), "Changing preview mode never edits simulator materials");
    }

    #[test]
    fn material_tabs_preview_the_whole_selected_linkset_without_editing_assets_or_rebuilding_unchanged_frames() {
        let (mut world, mut tool, settings, [root, child, other]) = fixture();
        let before: Vec<_> = [root, child, other]
            .map(|idx| {
                let o = world.objects.get(idx).unwrap();
                (o.te.clone(), o.extra.clone(), world.gltf_overrides[&o.key].clone())
            })
            .into();
        tool.edit_mode = EditMode::Face;
        tool.faces.insert(world.objects.get(root).unwrap().key, vec![0]);
        tick(&mut tool, &mut world, &settings);
        for idx in [root, child] {
            let o = world.objects.get(idx).unwrap();
            assert!(o.material_dirty);
            assert!((0..6).all(|face| o.visual_material(face).is_none()));
        }
        assert!(world.objects.get(other).unwrap().visual_material(0).is_some());
        for idx in [root, child, other] {
            world.objects.get_mut_untracked(idx).unwrap().material_dirty = false;
        }
        world.objects.take_touched(&mut Vec::new());
        tool.out.clear();
        tick(&mut tool, &mut world, &settings);
        assert!(world.objects.touched().is_empty(), "Steady previews must not touch scene records");
        assert!(tool.out.is_empty(), "Previewing sends no simulator edits");
        for tab in [1, 2, 0, 1] {
            tool.ui.tex_tab = tab;
            tool.update_material_preview(&mut world, &settings);
            for (i, idx) in [root, child, other].into_iter().enumerate() {
                let o = world.objects.get(idx).unwrap();
                assert_eq!(o.visual_material(5).is_none(), idx != other && tab == 0);
                assert_eq!(o.te, before[i].0);
                assert_eq!(o.extra, before[i].1);
                assert_eq!(world.gltf_overrides[&o.key], before[i].2);
            }
        }
        assert!(tool.out.is_empty());
    }

    #[test]
    fn leaving_materials_or_closing_the_editor_keeps_the_objects_last_mode() {
        for cause in 0..5 {
            let (mut world, mut tool, mut settings, [root, child, _]) = fixture();
            tick(&mut tool, &mut world, &settings);
            assert!(world.objects.get(root).unwrap().visual_material(0).is_none());
            match cause {
                0 => tool.tab = 6,
                1 => settings.expanded = false,
                2 => tool.set_tool(Tool::Create),
                3 => tool.close(&mut world, &mut settings),
                _ => tool.open = false,
            }
            tick(&mut tool, &mut world, &settings);
            for idx in [root, child] {
                assert!(world.objects.get(idx).unwrap().render.blinn_phong_preview);
                assert!(world.objects.get(idx).unwrap().visual_material(0).is_none());
            }
        }
    }

    #[test]
    fn preview_tracks_linked_selection_permissions_and_live_material_updates() {
        let (mut world, mut tool, settings, [root, child, other]) = fixture();
        tick(&mut tool, &mut world, &settings);
        tool.selection = vec![world.objects.get(child).unwrap().key];
        tool.selection_linked = true;
        tick(&mut tool, &mut world, &settings);
        assert!(world.objects.get(root).unwrap().visual_material(0).is_none());
        assert!(world.objects.get(child).unwrap().visual_material(0).is_none());
        world.objects.get_mut(root).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_MODIFY;
        tick(&mut tool, &mut world, &settings);
        tool.ui.tex_tab = 1;
        tick(&mut tool, &mut world, &settings);
        assert!(
            world.objects.get(child).unwrap().visual_material(0).is_none(),
            "No-mod cannot change the chosen mode"
        );
        world.objects.get_mut(root).unwrap().update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
        tool.ui.tex_tab = 0;
        tick(&mut tool, &mut world, &settings);
        let replacement = Uuid::from_u128(0xdef);
        world.objects.get_mut(child).unwrap().extra.render_materials = vec![(0, replacement)];
        tick(&mut tool, &mut world, &settings);
        assert!(world.objects.get(child).unwrap().visual_material(0).is_none());
        tool.ui.tex_tab = 1;
        tick(&mut tool, &mut world, &settings);
        assert_eq!(world.objects.get(child).unwrap().visual_material(0), Some(replacement));
        tool.ui.tex_tab = 0;
        tool.selection = vec![world.objects.get(other).unwrap().key];
        tick(&mut tool, &mut world, &settings);
        assert_eq!(world.objects.get(child).unwrap().visual_material(0), Some(replacement));
        assert!(world.objects.get(other).unwrap().visual_material(0).is_none());
        tool.selection.clear();
        tick(&mut tool, &mut world, &settings);
        assert!(tool.material_preview_selection.is_empty());
        assert!(world.objects.get(other).unwrap().visual_material(0).is_none());
        assert!(world.objects.get(root).unwrap().visual_material(0).is_none());
    }

    #[test]
    fn preview_survives_simulator_renumbering_and_clears_on_region_exit() {
        let (mut world, mut tool, settings, [root, _, _]) = fixture();
        tick(&mut tool, &mut world, &settings);
        let (id, extra) = {
            let o = world.objects.get(root).unwrap();
            (o.full_id, o.extra.clone())
        };
        let mut update = crate::demo::selection_cycle_events()
            .into_iter()
            .find_map(|event| {
                if let aurora_net::NetEvent::ObjectUpdates { objects, .. } = event {
                    objects.into_iter().find(|o| o.full_id == id)
                } else {
                    None
                }
            })
            .unwrap();
        update.extra = extra;
        update.local_id = 99999;
        world.objects.upsert(42, update);
        let moved = world.objects.index_of_uuid(&id).unwrap();
        assert!(world.objects.get(moved).unwrap().render.blinn_phong_preview);
        tick(&mut tool, &mut world, &settings);
        assert!(world.objects.get(moved).unwrap().render.blinn_phong_preview);
        world.apply(aurora_net::NetEvent::MainRegionChanged { handle: 42 });
        tick(&mut tool, &mut world, &settings);
        assert!(!world.objects.get(moved).unwrap().render.blinn_phong_preview);
        assert!(world.objects.get(moved).unwrap().visual_material(0).is_some());
    }

    #[test]
    fn region_exit_clears_deselected_objects_and_does_not_reapply_an_unchanged_inspection() {
        let (mut world, mut tool, settings, [root, child, other]) = fixture();
        let here = world.main_region.unwrap();
        tick(&mut tool, &mut world, &settings);
        tool.selection = vec![world.objects.get(other).unwrap().key];
        tick(&mut tool, &mut world, &settings);
        world.apply(aurora_net::NetEvent::MainRegionChanged { handle: here });
        assert!(
            [root, child, other]
                .into_iter()
                .all(|idx| world.objects.get(idx).unwrap().visual_material(0).is_none())
        );
        world.apply(aurora_net::NetEvent::MainRegionChanged { handle: here + 256 });
        tick(&mut tool, &mut world, &settings);
        assert!(
            [root, child, other]
                .into_iter()
                .all(|idx| world.objects.get(idx).unwrap().visual_material(0).is_some())
        );
        // A fresh explicit choice after arrival may preview this object again.
        tool.ui.tex_tab = 1;
        tick(&mut tool, &mut world, &settings);
        tool.ui.tex_tab = 0;
        tick(&mut tool, &mut world, &settings);
        assert!(world.objects.get(other).unwrap().visual_material(0).is_none());
        world.apply(aurora_net::NetEvent::MainRegionChanged { handle: here });
        assert!(
            world.objects.get(other).unwrap().visual_material(0).is_some(),
            "Returning cannot resurrect a cached choice"
        );
    }

    #[test]
    fn disconnect_discards_the_last_modes_before_the_next_session() {
        let (mut world, mut tool, settings, [root, _, _]) = fixture();
        let id = world.objects.get(root).unwrap().full_id;
        tick(&mut tool, &mut world, &settings);
        tool.close(&mut world, &mut BuildSettings::default());
        assert!(world.objects.get(root).unwrap().visual_material(0).is_none());
        // App::back_to_login uses World::reset for both logout and a lost link.
        world.reset();
        for event in crate::demo::events().into_iter().chain(crate::demo::selection_cycle_events()) {
            world.apply(event);
        }
        let idx = world.objects.index_of_uuid(&id).unwrap();
        world.objects.get_mut(idx).unwrap().extra.render_materials = vec![(0, Uuid::from_u128(0xabc))];
        tick(&mut tool, &mut world, &settings);
        assert!(!world.objects.get(idx).unwrap().render.blinn_phong_preview);
        assert!(world.objects.get(idx).unwrap().visual_material(0).is_some());
    }
}
