//! Folder imports retain mesh families and separately selectable glTF materials.
use super::*;

pub(in crate::imports) fn load(root: &Path, prefs: &Preferences, options: import_texture::Options, sound_limit: u32) -> Vec<Entry> {
    let discovered = match folders::discover(root) {
        Ok(files) if !files.is_empty() => files,
        Ok(_) => {
            return vec![Entry::new(
                root.to_owned(),
                Err(aurora_i18n::tr!("import-folders-empty").into()),
                options,
            )];
        }
        Err(reason) => return vec![Entry::new(root.to_owned(), Err(reason), options)],
    };
    let paths: Vec<_> = discovered.iter().map(|(path, _)| path.clone()).collect();
    let mut consumed = HashSet::new();
    let mut loaded_rows = Vec::new();
    for index in lods::order(&paths, prefs) {
        let (path, relative) = &discovered[index];
        if consumed.contains(&lods::identity(path)) {
            continue;
        }
        let mut loaded = super::load_selected(path, Choice::Bulk, options, sound_limit, prefs, false, [true; 5])
            .unwrap_or_else(|error| vec![Entry::new(path.clone(), Err(load_error(error)), options)]);
        for entry in &mut loaded {
            entry.source_folders.clone_from(relative);
            if let Some(Source::Mesh(draft)) = &entry.source {
                let own = lods::identity(path);
                for companion in draft.lods[..3]
                    .iter()
                    .filter(|lod| lod.file.is_some())
                    .filter_map(|lod| lod.path.as_ref())
                    .chain(draft.physics_path.iter().filter(|_| draft.physics_file.is_some()))
                {
                    let companion = lods::identity(companion);
                    if companion != own {
                        consumed.insert(companion);
                    }
                }
            }
        }
        loaded_rows.push((index, loaded));
    }
    loaded_rows.sort_by_key(|(index, _)| *index);
    loaded_rows.into_iter().flat_map(|(_, rows)| rows).collect()
}

#[cfg(test)]
mod tests;
