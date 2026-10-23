//! Native files and codec work are prepared off the frame thread. Opening the
//! existing upload inspector never starts a paid upload automatically.
use super::*;
use std::path::PathBuf;

pub struct Prepared {
    pub name: String,
    pub bytes: Arc<Vec<u8>>,
    pub animation: Arc<Animation>,
}
#[derive(Clone, Copy)]
pub(super) enum Action {
    Save,
    Import,
}
pub(super) enum Output {
    Preview {
        revision: u64,
        start: u32,
        end: u32,
        result: Result<Arc<BoundAnim>, &'static str>,
    },
    Prepared(Prepared),
    Saved(PathBuf),
    Cancelled,
    Failed(String),
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
pub(super) fn start(jobs: &Jobs, ctx: &egui::Context, tx: crossbeam_channel::Sender<Output>, document: Arc<Document>, action: Action) {
    let ctx = ctx.clone();
    jobs.execute(move || {
        let work = || -> Result<Output, String> {
            let animation = document.animation().map_err(|k| aurora_i18n::text(k).to_owned())?;
            let bytes = aurora_assets::import_animation::encode(&animation).map_err(|e| e.to_string())?;
            let animation = aurora_assets::anim::parse_animation_upload(&bytes).map_err(|e| e.to_string())?;
            let name = basename(&document.name);
            Ok(match action {
                Action::Import => Output::Prepared(Prepared {
                    name,
                    bytes: Arc::new(bytes),
                    animation: Arc::new(animation),
                }),
                Action::Save => {
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
