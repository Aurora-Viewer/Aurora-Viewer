//! Import of an appearance XML file (`linden_genepool` 1.0, the format the
//! Legacy Viewer exports and shape websites produce) into the draft of the
//! shape, physics or body part / clothing editor.
//!
//! Port of LLPanelEditWearable::onClickedImportBtnCallback
//! (indra/newview/llpaneleditwearable.cpp, originally LGPL 2.1): the root must
//! be `linden_genepool` version 1.0; each `<param id value>` of the first
//! `<archetype>` sets the weight of the parameter the edited wearable carries,
//! clamped to its range; the others are ignored; a malformed `<param>` is
//! reported and the rest still applies. Textures in the file are ignored, as
//! there. The weights only change the unsaved draft: Save keeps them.
//!
//! The native dialog, the bounded read and the parse run on a worker thread,
//! like the outfit JSON import (`ui/appearance/json.rs`).
use super::Draft;
use crate::scene::shape::ShapeParams;
use std::{
    io::Read,
    sync::mpsc::{Receiver, TryRecvError},
};
use uuid::Uuid;

/// A complete Legacy Viewer appearance dump is about 25 KB.
const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, PartialEq)]
pub struct Archetype {
    /// `(id, weight)` in file order.
    pub params: Vec<(i32, f32)>,
    /// `<param>` elements without a readable id or finite value.
    pub errors: usize,
}

#[derive(Debug, PartialEq)]
pub enum Error {
    /// Not XML, not `linden_genepool`, or not version 1.0
    /// (ShapeImportVersionFail).
    NotAvatarFile,
    /// No `<archetype>`, unreadable or too large (ShapeImportGenericFail).
    Failed,
}

impl Error {
    pub fn message(&self) -> &'static str {
        match self {
            Error::NotAvatarFile => aurora_i18n::tr!("appearance-xml-not-avatar"),
            Error::Failed => aurora_i18n::tr!("appearance-xml-failed"),
        }
    }
}

pub fn parse(bytes: &[u8]) -> Result<Archetype, Error> {
    let root = aurora_llsd::xml::parse(bytes).map_err(|_| Error::NotAvatarFile)?;
    if root.name != "linden_genepool" || root.attr("version").map(str::trim) != Some("1.0") {
        return Err(Error::NotAvatarFile);
    }
    let archetype = root.child("archetype").ok_or(Error::Failed)?;
    let mut out = Archetype {
        params: Vec::new(),
        errors: 0,
    };
    for param in archetype.elements().filter(|e| e.name == "param") {
        let id = param.attr("id").and_then(|v| v.trim().parse::<i32>().ok());
        let value = param
            .attr("value")
            .and_then(|v| v.trim().parse::<f32>().ok())
            .filter(|v| v.is_finite());
        match (id, value) {
            (Some(id), Some(value)) => out.params.push((id, value)),
            _ => out.errors += 1,
        }
    }
    Ok(out)
}

/// Set the weights the file gives for the parameters of `kind`, clamped to
/// their range like LLVisualParam::setWeight. Returns how many it set.
pub fn apply(archetype: &Archetype, kind: u8, lad: &ShapeParams, params: &mut std::collections::BTreeMap<i32, f32>) -> usize {
    let mut count = 0;
    for &(id, value) in &archetype.params {
        if let Some((min, max)) = lad.wearable_range(id, kind) {
            params.insert(id, value.clamp(min, max));
            count += 1;
        }
    }
    count
}

/// The message after an import: how many weights were set and, as the
/// Legacy Viewer's ShapeImportGenericFail, whether some could not be read.
pub fn summary(count: usize, errors: usize) -> String {
    if count == 0 {
        aurora_i18n::tr!("appearance-xml-none").into()
    } else if errors > 0 {
        aurora_i18n::tr!("appearance-xml-partial", count = count, errors = errors)
    } else {
        aurora_i18n::tr!("appearance-xml-imported", count = count)
    }
}

enum Outcome {
    Read(Archetype),
    Failed(Error),
    Cancelled,
}

/// One import at a time per editor, tied to the draft it was started for:
/// a reply for another item or asset (closed, saved, replaced) is dropped.
#[derive(Default)]
pub struct Import {
    job: Option<((Uuid, Uuid), Receiver<Outcome>)>,
}

impl Import {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Ask for a file with the native dialog, then read it.
    pub fn start(&mut self, ctx: &egui::Context, draft: &Draft) {
        self.spawn(ctx, draft, None);
    }

    /// Read `path` without the dialog (`AURORA_DEMO_SHAPE_XML`).
    pub fn start_file(&mut self, ctx: &egui::Context, draft: &Draft, path: std::path::PathBuf) {
        self.spawn(ctx, draft, Some(path));
    }

    fn spawn(&mut self, ctx: &egui::Context, draft: &Draft, path: Option<std::path::PathBuf>) {
        if self.busy() {
            return;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("aurora-appearance-xml".into()).spawn(move || {
            let path = path.or_else(|| {
                rfd::FileDialog::new()
                    .set_title(aurora_i18n::tr!("appearance-xml-import"))
                    .add_filter(aurora_i18n::tr!("appearance-xml-file-type"), &["xml"])
                    .pick_file()
            });
            let outcome = match path {
                None => Outcome::Cancelled,
                Some(path) => match read(&path) {
                    Ok(archetype) => Outcome::Read(archetype),
                    Err(error) => Outcome::Failed(error),
                },
            };
            let _ = sender.send(outcome);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.job = Some(((draft.item.id, draft.item.asset_id), receiver)),
            Err(e) => log::warn!("cannot start the appearance XML worker: {e}"),
        }
    }

    /// Apply a finished import to `draft` when it is still the same item and
    /// may be modified. `Some(message)` when the editor has news to show;
    /// the caller previews the draft again when weights were set.
    pub fn poll(&mut self, draft: Option<&mut Draft>, modify: bool, lad: &ShapeParams) -> Option<(String, bool)> {
        let (scope, receiver) = self.job.as_ref()?;
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Outcome::Failed(Error::Failed),
        };
        let scope = *scope;
        self.job = None;
        let draft = draft.filter(|d| (d.item.id, d.item.asset_id) == scope && d.pending.is_none())?;
        match outcome {
            Outcome::Cancelled => None,
            Outcome::Failed(error) => Some((error.message().into(), false)),
            // LLPanelEditWearable greys Import out without modify rights.
            Outcome::Read(_) if !modify => Some((aurora_i18n::tr!("appearance-xml-failed").into(), false)),
            Outcome::Read(archetype) => {
                let kind = draft.edited.kind;
                let count = apply(&archetype, kind, lad, &mut draft.edited.params);
                log::info!(
                    "appearance XML import: {count} weights set, {} unreadable parameters",
                    archetype.errors
                );
                Some((summary(count, archetype.errors), count > 0))
            }
        }
    }
}

fn read(path: &std::path::Path) -> Result<Archetype, Error> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .and_then(|file| file.take(MAX_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| {
            log::warn!("appearance XML read failed: {e}");
            Error::Failed
        })?;
    if bytes.len() as u64 > MAX_BYTES {
        log::warn!("appearance XML import: the file is larger than {MAX_BYTES} bytes");
        return Err(Error::Failed);
    }
    parse(&bytes).inspect_err(|e| log::warn!("appearance XML import: {e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = r#"<?xml version="1.0" encoding="US-ASCII" standalone="yes"?>
<linden_genepool version="1.0">
<archetype name="???">
	<param id="1" name="Big_Brow" value="0.200"/>
	<param id="33" name="Height" value="9.000"/>
	<param id="80" name="male" value="0.000"/>
	<param id="166" name="Eyebrow Size" value="0.5"/>
	<param name="no id" value="0.5"/>
	<param id="2" name="broken" value="x"/>
	<texture te="0" uuid="00000000-0000-0000-0000-000000000000"/>
</archetype>
</linden_genepool>
"#;

    fn lad() -> &'static ShapeParams {
        static LIBRARY: std::sync::OnceLock<crate::scene::avatar::AvatarLibrary> = std::sync::OnceLock::new();
        &LIBRARY.get_or_init(crate::scene::avatar::AvatarLibrary::load).shape_params
    }

    #[test]
    fn a_legacy_dump_gives_its_params_in_order_and_counts_the_unreadable() {
        let archetype = parse(DUMP.as_bytes()).expect("dump");
        assert_eq!(archetype.params, vec![(1, 0.2), (33, 9.0), (80, 0.0), (166, 0.5)]);
        assert_eq!(archetype.errors, 2);
    }

    #[test]
    fn other_roots_versions_and_files_are_refused() {
        assert_eq!(parse(b"not xml"), Err(Error::NotAvatarFile));
        assert_eq!(parse(b"<llsd><map/></llsd>"), Err(Error::NotAvatarFile));
        assert_eq!(
            parse(br#"<linden_genepool version="2.0"><archetype/></linden_genepool>"#),
            Err(Error::NotAvatarFile)
        );
        assert_eq!(parse(br#"<linden_genepool version="1.0"/>"#), Err(Error::Failed));
    }

    #[test]
    fn only_the_edited_wearables_params_are_set_and_clamped() {
        let archetype = parse(DUMP.as_bytes()).expect("dump");
        let mut shape = std::collections::BTreeMap::new();
        // 1, 33 and 80 are shape parameters; 166 belongs to the hair.
        assert_eq!(apply(&archetype, 0, lad(), &mut shape), 3);
        let (_, height_max) = lad().wearable_range(33, 0).expect("height");
        assert_eq!(shape.get(&33), Some(&height_max));
        assert_eq!(shape.get(&1), Some(&0.2));
        assert_eq!(shape.get(&80), Some(&0.0));
        assert!(!shape.contains_key(&166));
        let mut hair = std::collections::BTreeMap::new();
        assert_eq!(apply(&archetype, 2, lad(), &mut hair), 1);
        assert_eq!(hair.keys().copied().collect::<Vec<_>>(), vec![166]);
    }
}
