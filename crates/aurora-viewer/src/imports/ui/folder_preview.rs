//! The result tree combines real inventory ancestry with virtual destination paths.
use super::*;
use std::collections::BTreeMap;

pub(super) struct Node {
    pub name: String,
    pub edit: Option<Vec<String>>,
}
#[derive(Default)]
pub(super) struct Preview {
    pub nodes: BTreeMap<Vec<String>, Node>,
    pub files: BTreeMap<Vec<String>, Vec<usize>>,
}

pub(super) fn build(ctx: &egui::Context, imports: &Imports, world: &crate::world::World) -> Preview {
    let order = crate::ui::inventory::view::picker_order(ctx, &world.inventory);
    let mut preview = Preview::default();
    for (index, entry) in imports
        .entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !entry.local && entry.selected)
    {
        let target = if imports.folders.active() {
            imports.folders.destination
        } else {
            entry.folder
        };
        let mut ancestry = Vec::new();
        let mut seen = HashSet::new();
        let mut current = target;
        while seen.insert(current)
            && let Some(folder) = world.inventory.folders.get(&current)
        {
            ancestry.push((current, crate::ui::inventory::folder_name(&world.inventory, current)));
            current = folder.info.parent;
        }
        ancestry.reverse();
        let mut key = Vec::new();
        if ancestry.is_empty() {
            key.push(format!("missing:{target}"));
            preview.nodes.entry(key.clone()).or_insert_with(|| Node {
                name: aurora_i18n::tr!("import-no-folder").into(),
                edit: None,
            });
        }
        for (id, name) in ancestry {
            key.push(format!("i:{:020}:{id}", order.rank(id)));
            preview.nodes.entry(key.clone()).or_insert(Node { name, edit: None });
        }
        if imports.folders.active()
            && let Some(path) = &entry.upload_path
        {
            for depth in 1..=path.len() {
                key.push(format!("p:{}", path[depth - 1]));
                preview.nodes.entry(key.clone()).or_insert_with(|| Node {
                    name: path[depth - 1].clone(),
                    edit: Some(path[..depth].to_vec()),
                });
            }
        }
        preview.files.entry(key).or_default().push(index);
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_follows_real_uuid_ancestry_and_shared_routing_instead_of_the_source_tree() {
        let ctx = egui::Context::default();
        let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.inventory.root = Uuid::from_u128(1);
        world.inventory.created_folders(
            [
                (1, 0, "Inventory"),
                (2, 1, "Objects"),
                (3, 1, "Textures"),
                (4, 2, "Pack"),
                (5, 3, "Pack"),
            ]
            .map(|(id, parent, name)| aurora_net::inventory::InvFolder {
                id: Uuid::from_u128(id),
                parent: Uuid::from_u128(parent),
                name: name.into(),
                type_default: -1,
                ..Default::default()
            })
            .into(),
        );
        let mut imports = Imports::default();
        for target in [4, 5] {
            let mut entry = Entry::new(
                "Fabric.png".into(),
                Ok(Source::Texture(crate::imports::tests::image(16, 16))),
                Default::default(),
            );
            entry.folder = Uuid::from_u128(target);
            entry.upload_path = Some(vec!["Source".into()]);
            imports.entries.push(entry);
        }
        let preview = build(&ctx, &imports, &world);
        let chains: Vec<_> = preview
            .files
            .keys()
            .map(|path| {
                (1..=path.len())
                    .map(|depth| preview.nodes[&path[..depth]].name.as_str())
                    .collect::<Vec<_>>()
            })
            .collect();
        let root = aurora_i18n::tr!("common-my-inventory");
        assert!(chains.contains(&vec![root, "Objects", "Pack"]), "{chains:?}");
        assert!(chains.contains(&vec![root, "Textures", "Pack"]), "{chains:?}");
        assert_eq!(preview.files.len(), 2, "equal names keep their actual UUID ancestry");
        assert!(preview.nodes.values().all(|node| node.edit.is_none()));

        imports.folders.enabled = true;
        imports.folders.destination = Uuid::from_u128(2);
        for entry in &mut imports.entries {
            entry.upload_path = Some(vec!["New".into()]);
        }
        let preview = build(&ctx, &imports, &world);
        assert_eq!(preview.files.len(), 1);
        let (path, files) = preview.files.first_key_value().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(preview.nodes[path].edit, Some(vec!["New".into()]));
        assert_eq!(preview.nodes[&path[..2]].name, "Objects");
        for entry in &mut imports.entries {
            entry.upload_path = Some(Vec::new());
        }
        let flat = build(&ctx, &imports, &world);
        let path = flat.files.first_key_value().unwrap().0;
        assert_eq!(flat.nodes[path].name, "Objects");
        assert!(flat.nodes.values().all(|node| node.edit.is_none()));
    }
}
