//! Configured companion naming follows LLModelPreview::lookupLODModelFiles
//! (indra/newview/llmodelpreview.cpp).
use super::*;
use std::collections::BTreeSet;

pub(super) fn candidates(path: &Path, prefs: &Preferences) -> Vec<(usize, PathBuf)> {
    let extension = path.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if !matches!(extension.as_str(), "dae" | "gltf" | "glb") {
        return Vec::new();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let high = &prefs.mesh_suffixes[3];
    let base = if high.is_empty() {
        stem.as_ref()
    } else {
        stem.strip_suffix(&format!("_{high}")).unwrap_or(&stem)
    };
    [2, 1, 0, 4]
        .into_iter()
        .filter_map(|level| {
            let suffix = &prefs.mesh_suffixes[level];
            if suffix.is_empty() || suffix.contains(['/', '\\', ':']) {
                return None;
            }
            let candidate = path.with_file_name(format!("{base}_{suffix}.{extension}"));
            (candidate != path).then_some((level, candidate))
        })
        .collect()
}

pub(super) fn identity(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_owned())
}

/// Parents load before possible companions; failed associations are never hidden.
pub(super) fn order(paths: &[PathBuf], prefs: &Preferences) -> Vec<usize> {
    let indices: HashMap<_, _> = paths.iter().enumerate().map(|(i, p)| (identity(p), i)).collect();
    let mut children = vec![BTreeSet::new(); paths.len()];
    let mut incoming = vec![0usize; paths.len()];
    for (parent, path) in paths.iter().enumerate() {
        for (_, candidate) in candidates(path, prefs) {
            if let Some(&child) = indices.get(&identity(&candidate))
                && child != parent
                && children[parent].insert(child)
            {
                incoming[child] += 1;
            }
        }
    }
    let mut ready: BTreeSet<_> = incoming
        .iter()
        .enumerate()
        .filter_map(|(i, count)| (*count == 0).then_some(i))
        .collect();
    let mut ordered = Vec::with_capacity(paths.len());
    while let Some(index) = ready.pop_first() {
        ordered.push(index);
        for &child in &children[index] {
            incoming[child] -= 1;
            if incoming[child] == 0 {
                ready.insert(child);
            }
        }
    }
    // Conflicting custom suffixes cannot discard a source or loop forever.
    ordered.extend((0..paths.len()).filter(|index| incoming[*index] > 0));
    ordered
}
