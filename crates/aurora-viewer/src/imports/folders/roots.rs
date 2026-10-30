//! Optional, editable source-directory wrappers shared by every layout.

use super::*;

#[derive(Clone)]
pub struct SourceRoots {
    pub include: bool,
    names: HashMap<String, String>,
}

impl Default for SourceRoots {
    fn default() -> Self {
        Self {
            include: true,
            names: HashMap::new(),
        }
    }
}

impl SourceRoots {
    pub fn name(&self, source: &str) -> String {
        self.names.get(source).cloned().unwrap_or_else(|| folder_name(source))
    }

    pub(super) fn layout_path(&self, entry: &Entry, layout: Layout) -> Vec<String> {
        let mut path = Vec::new();
        if self.include
            && let Some(root) = entry.source_folders.first()
        {
            path.push(self.name(root));
        }
        match layout {
            Layout::Mirror => path.extend(entry.source_folders.iter().skip(1).map(|part| folder_name(part))),
            Layout::Flat => {}
            Layout::Type => path.push(folder_name(match entry.source.as_ref().map(Source::kind) {
                Some(Kind::Texture | Kind::Snapshot) => aurora_i18n::tr!("import-folders-textures"),
                Some(Kind::Sound) => aurora_i18n::tr!("import-folders-sounds"),
                Some(Kind::Animation) => aurora_i18n::tr!("import-folders-animations"),
                Some(Kind::Material) => aurora_i18n::tr!("import-folders-materials"),
                Some(Kind::Mesh) => aurora_i18n::tr!("import-folders-meshes"),
                None => aurora_i18n::tr!("import-folders-other"),
            })),
        }
        path
    }
}

impl Imports {
    pub fn source_roots(&self) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|e| !e.local)
            .filter_map(|e| e.source_folders.first().cloned())
            .collect()
    }

    pub fn rename_source_root(&mut self, source: &str, name: &str) -> bool {
        if self.busy() || self.confirm || !self.source_roots().contains(source) {
            return false;
        }
        let Some(path) = parse_path(name).filter(|p| p.len() == 1) else {
            return false;
        };
        let mut roots = self.folders.roots.clone();
        roots.names.insert(source.into(), path[0].clone());
        self.update_source_roots(roots);
        true
    }

    /// Keep manual inner paths while replacing only the known wrapper.
    pub(super) fn update_source_roots(&mut self, mut roots: SourceRoots) {
        let sources = self.source_roots();
        roots.names.retain(|source, _| sources.contains(source));
        let previous = &self.folders.roots;
        let mut changed = false;
        for entry in self.entries.iter_mut().filter(|e| !e.local) {
            let Some(source) = entry.source_folders.first() else { continue };
            let old = previous.include.then(|| previous.name(source));
            let new = roots.include.then(|| roots.name(source));
            if old == new {
                continue;
            }
            let Some(path) = &mut entry.upload_path else { continue };
            if old.as_ref().is_some_and(|old| path.first() == Some(old)) {
                path.remove(0);
            }
            if let Some(new) = new {
                path.insert(0, new);
            }
            if let Some(Source::Mesh(draft)) = &mut entry.source {
                draft.invalidate_quote();
                entry.preview_generation += 1;
            }
            changed = true;
        }
        self.folders.roots = roots;
        if changed {
            self.folders.editing = None;
            self.folders.collapsed.clear();
        }
    }
}

#[cfg(test)]
mod tests;
