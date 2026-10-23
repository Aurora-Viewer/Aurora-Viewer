//! Row actions address their own project without changing the working pose.
use super::*;

pub(in crate::ik_edit::project) enum Management {
    Rename(String),
    Duplicate(String),
    File,
    Notecard,
    Delete,
}

pub(super) fn execute(
    root: &Path,
    current: Document,
    id: Uuid,
    action: Management,
    default_name: String,
    default_profile: String,
) -> Result<Output, String> {
    let source = document(root, current, id)?;
    let command = match action {
        Management::Rename(name) => Command::Rename(name),
        Management::Duplicate(name) => Command::Duplicate(name),
        Management::Delete => Command::Delete(id),
        Management::File => {
            return export_file(source, || {
                rfd::FileDialog::new()
                    .add_filter("Aurora IK", &["json"])
                    .set_file_name("project.aurora-ik.json")
                    .save_file()
            });
        }
        Management::Notecard => return Ok(Output::LibraryNotecard(source.encode()?)),
    };
    match super::execute(root, source, command, default_name, default_profile)? {
        Output::LibraryApplied(bytes) => Ok(Output::LibraryCreated(bytes)),
        output => Ok(output),
    }
}

pub(super) fn document(root: &Path, current: Document, id: Uuid) -> Result<Document, String> {
    if current.project.as_ref().is_some_and(|p| p.id == id) {
        Ok(current)
    } else {
        Document::decode(&load(root, id)?)
    }
}

pub(super) fn export_file(source: Document, choose: impl FnOnce() -> Option<PathBuf>) -> Result<Output, String> {
    let bytes = source.encode()?;
    let Some(path) = choose() else { return Ok(Output::Cancelled) };
    write(&path, &bytes)?;
    Ok(Output::Saved)
}

impl Editor {
    pub(super) fn begin_library_row_edit(&mut self, entry: &Identity, kind: EditKind) {
        self.begin_project_creation();
        let name = if matches!(kind, EditKind::Duplicate) {
            self.project.library.unique(&entry.name)
        } else {
            entry.name.clone()
        };
        self.project.library.edit = Some(Edit {
            kind,
            target: Some(entry.id),
            name,
            focus: true,
        });
    }
}

#[cfg(test)]
mod tests;
