//! Export of the draft of the shape, physics or body part / clothing editor
//! to an appearance XML file (`linden_genepool` 1.0), the file
//! `xml_import` and the Legacy Viewer's Import button read back.
//!
//! The format of LLVOAvatar::dumpArchetypeXML with `group_by_wearables` and
//! dump_visual_param (indra/newview/llvoavatar.cpp, originally LGPL 2.1), for
//! the one wearable the editor holds: a `<param id name value wearable
//! group>` for every tweakable parameter of its type (LLViewerVisualParam::
//! isTweakable: groups 0 and 2; the animated ones the physics drives are
//! left out, as there), with the draft's weight (its default when the asset
//! has none) at three decimals.
//!
//! Stricter than the Legacy Viewer (whose dump checks no permission): the
//! button needs modify rights on the item
//! (`actions::appearance_xml_export_allowed`), and the file never carries a
//! texture, joint or mesh id, whatever the rights.
//!
//! The native dialog and the write run on a worker thread, like the outfit
//! JSON export (`ui/appearance/json.rs`); the text is built on the click,
//! a few hundred short lines.
use super::Draft;
use crate::scene::shape::ShapeParams;
use std::{
    collections::BTreeMap,
    fmt::Write as _,
    path::PathBuf,
    sync::mpsc::{Receiver, TryRecvError},
};

/// The XML text of a wearable of `kind` named `name` with these weights.
pub fn document(name: &str, kind: u8, weights: &BTreeMap<i32, f32>, lad: &ShapeParams) -> String {
    let wearable = crate::world::inventory::wearable::type_name(kind).unwrap_or("invalid");
    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n");
    out.push_str("<linden_genepool version=\"1.0\">\n");
    let _ = writeln!(out, "\n\t<archetype name=\"{}\">", escape(name));
    let _ = writeln!(out, "\n\t\t<!-- wearable: {wearable} -->");
    for param in tweakable(lad, kind) {
        let value = weights
            .get(&param.id)
            .copied()
            .filter(|v| v.is_finite())
            .unwrap_or(param.default)
            .clamp(param.min, param.max);
        let _ = writeln!(
            out,
            "\t\t<param id=\"{}\" name=\"{}\" value=\"{value:.3}\" wearable=\"{wearable}\" group=\"{}\"/>",
            param.id,
            escape(param.name),
            param.group
        );
    }
    out.push_str("\t</archetype>\n</linden_genepool>\n");
    out
}

fn tweakable(lad: &ShapeParams, kind: u8) -> impl Iterator<Item = crate::scene::shape::WearableParam<'_>> {
    lad.wearable_params(kind).filter(|p| matches!(p.group, 0 | 2))
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // XML 1.0 refuses most control characters, even escaped.
            c if c.is_control() && !matches!(c, '\t' | '\n' | '\r') => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

enum Outcome {
    Saved(String),
    Failed,
    Cancelled,
}

/// One export at a time per editor.
#[derive(Default)]
pub struct Export {
    job: Option<Receiver<Outcome>>,
}

impl Export {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Ask where to save with the native dialog, then write the draft there.
    /// `allowed` is `Draft::can_export_xml`: a refusal comes back as the
    /// message to show.
    pub fn start(&mut self, ctx: &egui::Context, draft: &Draft, allowed: bool, lad: &ShapeParams) -> Result<(), String> {
        self.spawn(ctx, draft, allowed, lad, None)
    }

    /// Write to `path` without the dialog (`AURORA_DEMO_SHAPE_XML_EXPORT`),
    /// under the same permission rule.
    pub fn start_file(
        &mut self,
        ctx: &egui::Context,
        draft: &Draft,
        allowed: bool,
        lad: &ShapeParams,
        path: PathBuf,
    ) -> Result<(), String> {
        self.spawn(ctx, draft, allowed, lad, Some(path))
    }

    fn spawn(&mut self, ctx: &egui::Context, draft: &Draft, allowed: bool, lad: &ShapeParams, path: Option<PathBuf>) -> Result<(), String> {
        if !allowed {
            log::info!("appearance XML export refused: no modify rights on the item");
            return Err(aurora_i18n::tr!("appearance-xml-export-refused").into());
        }
        if self.busy() {
            return Ok(());
        }
        let text = document(&draft.item.name, draft.edited.kind, &draft.edited.params, lad);
        let file_name = format!("{}.xml", crate::world::outfit_backup::file_stem(&draft.item.name, "appearance"));
        let (sender, receiver) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("aurora-appearance-xml-export".into())
            .spawn(move || {
                let path = path.or_else(|| {
                    rfd::FileDialog::new()
                        .set_title(aurora_i18n::tr!("appearance-xml-export"))
                        .add_filter(aurora_i18n::tr!("appearance-xml-file-type"), &["xml"])
                        .set_file_name(file_name)
                        .save_file()
                });
                let outcome = match path {
                    None => Outcome::Cancelled,
                    Some(path) => match crate::atomic_file::write(&path, text.as_bytes()) {
                        Ok(()) => {
                            log::info!("appearance XML exported ({} bytes)", text.len());
                            Outcome::Saved(path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                        }
                        Err(e) => {
                            log::warn!("appearance XML export failed: {e}");
                            Outcome::Failed
                        }
                    },
                };
                let _ = sender.send(outcome);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.job = Some(receiver),
            Err(e) => {
                log::warn!("cannot start the appearance XML export worker: {e}");
                return Err(aurora_i18n::tr!("appearance-xml-export-failed").into());
            }
        }
        Ok(())
    }

    /// The message to show once the export finished, if any.
    pub fn poll(&mut self) -> Option<String> {
        let outcome = match self.job.as_ref()?.try_recv() {
            Ok(outcome) => outcome,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Outcome::Failed,
        };
        self.job = None;
        match outcome {
            Outcome::Cancelled => None,
            Outcome::Failed => Some(aurora_i18n::tr!("appearance-xml-export-failed").into()),
            Outcome::Saved(file) => Some(aurora_i18n::tr!("appearance-xml-exported", file = file)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lad() -> &'static ShapeParams {
        static LIBRARY: std::sync::OnceLock<crate::scene::avatar::AvatarLibrary> = std::sync::OnceLock::new();
        &LIBRARY.get_or_init(crate::scene::avatar::AvatarLibrary::load).shape_params
    }

    #[test]
    fn an_export_reads_back_as_the_same_weights_of_the_same_wearable() {
        let (_, height_max) = lad().wearable_range(33, 0).expect("height");
        let weights = BTreeMap::from([(33, height_max + 5.0), (1, 0.2), (166, 0.7)]);
        let text = document("My <shape> & \"co\"", 0, &weights, lad());
        let archetype = super::super::xml_import::parse(text.as_bytes()).expect("own export");
        assert_eq!(archetype.errors, 0);
        let ids: Vec<i32> = archetype.params.iter().map(|(id, _)| *id).collect();
        let expected: Vec<i32> = tweakable(lad(), 0).map(|p| p.id).collect();
        assert_eq!(ids, expected, "every tweakable shape parameter, by id");
        assert!(lad().wearable_params(0).any(|p| p.group == 1), "the shape has animated ones");
        assert!(!ids.contains(&166), "a hair parameter is not a shape's");
        let get = |id| archetype.params.iter().find(|(i, _)| *i == id).map(|(_, v)| *v);
        assert!((get(33).expect("height") - height_max).abs() < 0.001, "clamped to its range");
        assert!((get(1).expect("brow") - 0.2).abs() < 0.001);
        let unset = tweakable(lad(), 0).find(|p| !weights.contains_key(&p.id)).expect("another");
        assert!((get(unset.id).expect("default") - unset.default).abs() < 0.001, "the default");
        // Imported back, the shape gets the same weights.
        let mut back = BTreeMap::new();
        assert_eq!(super::super::xml_import::apply(&archetype, 0, lad(), &mut back), expected.len());
    }

    #[test]
    fn an_export_names_the_item_and_carries_no_id() {
        let text = document("A & B", 2, &BTreeMap::new(), lad());
        assert!(text.contains("<archetype name=\"A &amp; B\">"));
        assert!(text.contains("wearable=\"hair\""));
        assert!(!text.contains("uuid"), "no texture id");
        assert!(!text.contains("<texture"));
        assert!(!text.contains("mesh_id"));
        assert!(text.contains("id=\"166\""), "a hair parameter");
    }

    #[test]
    fn control_characters_do_not_break_the_file() {
        let text = document("a\u{1}b", 0, &BTreeMap::new(), lad());
        assert!(super::super::xml_import::parse(text.as_bytes()).is_ok());
    }
}
