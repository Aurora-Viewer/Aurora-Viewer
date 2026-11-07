//! Native files and codec work are prepared off the frame thread. Opening the
//! existing upload inspector never starts a paid upload automatically.
use super::*;
use std::{io::Read, path::PathBuf};

pub struct Prepared {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
    pub animation: Arc<Animation>,
}
#[derive(Clone, Copy)]
pub(super) enum Action {
    Load,
    LoadProject,
    SaveProject,
    Save { last_key_only: bool },
    Import { last_key_only: bool },
}
pub(super) enum Output {
    Preview {
        revision: u64,
        start: u32,
        end: u32,
        result: Result<Arc<BoundAnim>, &'static str>,
    },
    Prepared(Prepared),
    Loaded(Document),
    Saved(PathBuf),
    Cancelled,
    Failed(String),
}
pub(super) fn read_bounded(mut reader: impl Read, maximum: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take(maximum.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > maximum {
        return Err(aurora_i18n::text("anim-edit-file-too-large").to_owned());
    }
    Ok(bytes)
}
fn read_file(path: impl AsRef<std::path::Path>) -> Result<Vec<u8>, String> {
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    read_bounded(file, document::MAX_PROJECT_BYTES)
}
pub(super) fn load_document(
    bytes: &[u8],
    extension: &str,
    rig: &Rig,
    skeleton: Option<&aurora_assets::Skeleton>,
) -> Result<Document, String> {
    let animation = if extension.eq_ignore_ascii_case("bvh") {
        let skeleton = skeleton.ok_or_else(|| aurora_i18n::text("anim-edit-bvh-unavailable").to_owned())?;
        aurora_assets::import_animation::parse_bvh_for_edit(bytes, skeleton)
            .map_err(|error| error.to_string())?
            .animation
    } else if extension.eq_ignore_ascii_case("anim") {
        aurora_assets::anim::parse_animation_upload(bytes).map_err(|error| error.to_string())?
    } else {
        return Err(aurora_i18n::text("anim-edit-invalid").to_owned());
    };
    Document::from_animation(animation, rig).map_err(|key| aurora_i18n::text(key).to_owned())
}
pub fn basename(name: &str) -> String {
    let name: String = name
        .chars()
        .map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let name = name.trim().trim_end_matches('.');
    if name.is_empty() {
        "animation".into()
    } else {
        name.chars().take(100).collect()
    }
}
pub(super) fn start(
    jobs: &Jobs,
    ctx: &egui::Context,
    tx: crossbeam_channel::Sender<Output>,
    document: Arc<Document>,
    action: Action,
    rig: Arc<Rig>,
    skeleton: Option<Arc<aurora_assets::Skeleton>>,
) {
    let ctx = ctx.clone();
    jobs.execute(move || {
        let work = || -> Result<Output, String> {
            if matches!(action, Action::Load) {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter(aurora_i18n::tr!("anim-edit-file-filter"), &["anim"])
                    .add_filter(aurora_i18n::tr!("anim-edit-bvh-filter"), &["bvh"])
                    .pick_file()
                else {
                    return Ok(Output::Cancelled);
                };
                let bytes = read_file(&path)?;
                let extension = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
                return load_document(&bytes, extension, &rig, skeleton.as_deref()).map(Output::Loaded);
            }
            if matches!(action, Action::LoadProject) {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter(aurora_i18n::tr!("anim-edit-project-filter"), &["auroraanim"])
                    .pick_file()
                else {
                    return Ok(Output::Cancelled);
                };
                let bytes = read_file(path)?;
                return Document::from_project_json(&bytes, &rig)
                    .map(Output::Loaded)
                    .map_err(|key| aurora_i18n::text(key).to_owned());
            }
            if matches!(action, Action::SaveProject) {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter(aurora_i18n::tr!("anim-edit-project-filter"), &["auroraanim"])
                    .set_file_name(format!("{}.auroraanim", basename(&document.name)))
                    .save_file()
                else {
                    return Ok(Output::Cancelled);
                };
                std::fs::write(&path, document.project_json().map_err(|key| aurora_i18n::text(key).to_owned())?)
                    .map_err(|e| e.to_string())?;
                return Ok(Output::Saved(path));
            }
            let last_key_only = match action {
                Action::Save { last_key_only } | Action::Import { last_key_only } => last_key_only,
                _ => false,
            };
            let animation = document
                .animation_for_export(last_key_only)
                .map_err(|k| aurora_i18n::text(k).to_owned())?;
            let bytes = aurora_assets::import_animation::encode(&animation).map_err(|e| e.to_string())?;
            let animation = aurora_assets::anim::parse_animation_upload(&bytes).map_err(|e| e.to_string())?;
            let name = basename(&document.name);
            Ok(match action {
                Action::Load => unreachable!("file loading returned above"),
                Action::LoadProject | Action::SaveProject => unreachable!("project file operation returned above"),
                Action::Import { .. } => Output::Prepared(Prepared {
                    name,
                    bytes: Arc::new(bytes),
                    animation: Arc::new(animation),
                }),
                Action::Save { .. } => {
                    let Some(path) = rfd::FileDialog::new()
                        .add_filter(aurora_i18n::tr!("anim-edit-file-filter"), &["anim"])
                        .set_file_name(format!("{name}.anim"))
                        .save_file()
                    else {
                        return Ok(Output::Cancelled);
                    };
                    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
                    Output::Saved(path)
                }
            })
        };
        let output = work().unwrap_or_else(Output::Failed);
        let _ = tx.send(output);
        ctx.request_repaint();
    });
}
