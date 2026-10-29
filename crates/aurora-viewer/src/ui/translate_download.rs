//! The dialog that downloads the translation model on first use
//! (`crate::translate`): what it is and its size, then its progress.

use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind, action_button};
use aurora_translate::{DownloadError, DownloadState, ModelEntry};
use egui::RichText;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Download,
    /// "Not now", "Cancel" or "Close": the conversation stays untranslated.
    Cancel,
}

fn gb(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1e9)
}

pub fn show(ctx: &egui::Context, p: &Palette, model: &ModelEntry, state: Option<&DownloadState>) -> Option<Choice> {
    let mut choice = None;
    let response = widgets::Modal::new(egui::Id::unique("translate_download")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.label(
            RichText::new(aurora_i18n::tr!("translate-download-title"))
                .size(17.0)
                .strong()
                .color(p.ink),
        );
        ui.add_space(6.0);
        let size = egui::Vec2::new(0.0, 26.0);
        match state {
            None | Some(DownloadState::Cancelled) => {
                ui.label(aurora_i18n::tr!(
                    "translate-download-text",
                    model = model.label,
                    size = gb(model.bytes)
                ));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if action_button(
                        ui,
                        p,
                        "download-filled",
                        aurora_i18n::tr!("translate-download"),
                        ActionKind::Primary,
                        size,
                    )
                    .clicked()
                    {
                        choice = Some(Choice::Download);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("translate-not-now"), ActionKind::Flat, size).clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            }
            Some(DownloadState::Running { done, total }) => {
                let text = aurora_i18n::tr!("translate-download-progress", done = gb(*done), total = gb(*total));
                widgets::progress(
                    ui,
                    p,
                    "translate-download",
                    *done as f32 / (*total).max(1) as f32,
                    380.0,
                    &text,
                    None,
                );
                ui.add_space(8.0);
                if action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    choice = Some(Choice::Cancel);
                }
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
            Some(DownloadState::Verifying | DownloadState::Ready(_)) => {
                widgets::progress(
                    ui,
                    p,
                    "translate-download",
                    1.0,
                    380.0,
                    aurora_i18n::tr!("translate-download-checking"),
                    None,
                );
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
            Some(DownloadState::Failed(e)) => {
                let why = match e {
                    DownloadError::NoSpace { needed } => aurora_i18n::tr!("translate-download-no-space", size = gb(*needed)),
                    DownloadError::Checksum => aurora_i18n::tr!("translate-download-corrupt").to_string(),
                    DownloadError::Http(m) | DownloadError::Io(m) => aurora_i18n::tr!("translate-download-failed", reason = m.clone()),
                };
                ui.label(RichText::new(why).color(p.danger));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if action_button(ui, p, "refresh", aurora_i18n::tr!("translate-retry"), ActionKind::Primary, size).clicked() {
                        choice = Some(Choice::Download);
                    }
                    if action_button(ui, p, "x", aurora_i18n::tr!("common-close"), ActionKind::Flat, size).clicked() {
                        choice = Some(Choice::Cancel);
                    }
                });
            }
        }
    });
    if response.should_close() && !matches!(state, Some(DownloadState::Running { .. } | DownloadState::Verifying)) {
        choice.get_or_insert(Choice::Cancel);
    }
    choice
}
