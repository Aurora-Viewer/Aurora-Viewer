//! Individual prim metadata follows FSAreaSearch::requestObjectProperties
//! (indra/newview/fsareasearch.cpp): select then immediately deselect.
//! Keep the user's server-side selection and local manipulators intact.
use super::*;

impl BuildTool {
    pub(crate) fn prim_properties_query(&self, world: &World, key: ObjKey) -> Vec<NetCommand> {
        if !world
            .objects
            .index_of(&key)
            .and_then(|i| world.objects.get(i))
            .is_some_and(|o| !o.is_avatar())
        {
            return Vec::new();
        }
        // Every member's exact name is needed for uniqueness, including the
        // attachment root. Do not depend on a family/hover response for it.
        let mut result = vec![NetCommand::Build(BuildCmd::Select {
            handle: key.region,
            local_ids: vec![key.local_id],
        })];
        let selected = self.wire_ids(world, &self.selection, self.selection_linked);
        if !selected.get(&key.region).is_some_and(|ids| ids.contains(&key.local_id)) {
            result.push(NetCommand::Build(BuildCmd::Deselect {
                handle: key.region,
                local_ids: vec![key.local_id],
            }));
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn individual_property_reads_preserve_linkset_and_linked_edit_selection() {
        let (mut world, mut tool, mut settings) = super::super::selection_tests::seated_fixture(false);
        let root = world.objects.iter().find(|(_, o)| o.key.local_id == 950).unwrap().0;
        let child = world.objects.iter().find(|(_, o)| o.key.local_id == 951).unwrap().0;
        let key = world.objects.get(child).unwrap().key;
        for linked in [false, true] {
            tool.set_edit_linked(&mut world, &mut settings, linked);
            tool.click_select(&mut world, &settings, Some(if linked { child } else { root }), false);
            let selection = tool.selection.clone();
            let queued = tool.out.len();
            let command = tool.prim_properties_query(&world, key);
            assert!(matches!(&command[..], [NetCommand::Build(BuildCmd::Select { handle, local_ids })]
                if *handle == key.region && local_ids == &[key.local_id]));
            assert_eq!(tool.selection, selection);
            assert_eq!(tool.out.len(), queued, "read must not enqueue an Edit action");
            tool.deselect_all(&world);
            let commands = tool.prim_properties_query(&world, key);
            assert!(matches!(&commands[..], [
                NetCommand::Build(BuildCmd::Select { handle, local_ids: a }),
                NetCommand::Build(BuildCmd::Deselect { handle: other, local_ids: b }),
            ] if *handle == key.region && handle == other && a == &[key.local_id] && a == b));
            assert!(tool.selection.is_empty());
        }
    }

    #[test]
    fn property_reads_include_roots_preserve_selected_roots_and_reject_missing_objects_and_avatars() {
        let (mut world, mut tool, settings) = super::super::selection_tests::seated_fixture(false);
        let index = world.objects.iter().find(|(_, o)| o.key.local_id == 950).unwrap().0;
        let key = world.objects.get(index).unwrap().key;
        assert!(matches!(&tool.prim_properties_query(&world, key)[..], [
            NetCommand::Build(BuildCmd::Select { handle, local_ids: a }),
            NetCommand::Build(BuildCmd::Deselect { handle: other, local_ids: b }),
        ] if *handle == key.region && handle == other && a == &[key.local_id] && a == b));
        tool.click_select(&mut world, &settings, Some(index), false);
        let selection = tool.selection.clone();
        assert!(matches!(&tool.prim_properties_query(&world, key)[..],
            [NetCommand::Build(BuildCmd::Select { local_ids, .. })] if local_ids == &[key.local_id]));
        assert_eq!(tool.selection, selection);
        let avatar = world.objects.iter().find(|(_, o)| o.is_avatar()).unwrap().1;
        assert!(tool.prim_properties_query(&world, avatar.key).is_empty());
        let unknown = ObjKey {
            region: key.region,
            local_id: u32::MAX,
        };
        assert!(tool.prim_properties_query(&world, unknown).is_empty());
    }
}
