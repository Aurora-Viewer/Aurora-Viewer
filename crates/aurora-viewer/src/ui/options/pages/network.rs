//! Preferences › Network and cache: the bandwidth and the neighbouring
//! regions, the disk cache (its size, its folder: the Legacy Viewer's
//! Network & Files › CacheLocation, applied at the next start), the logs
//! and the Dev Toolbox.

use super::super::cx::{Cx, Nothing, field, fields};
use super::super::{Folder, FolderChoice, SLIDER_W, format_bytes, path_label};
use crate::ui::widgets::{self, ActionKind, action_button};
use egui::{RichText, Vec2};

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    cx.group(tr!("grids-list"), |cx| {
        cx.row(tr!("grids-manage"), tr!("grids-description"), &Nothing, |ui, _| {
            if ui.button(tr!("grids-manage")).clicked() {
                crate::ui::grids::request_open(ui.ctx());
            }
            false
        });
    });
    cx.group(tr!("ui-options-network"), |cx| {
        // (the step of the Legacy Viewer's slider, panel_preferences_setup.xml,
        // which stops at 3,000; ours goes up to what asks the most)
        cx.row(
            tr!("ui-options-maximum-bandwidth"),
            tr!("ui-options-maximum-bandwidth-hint"),
            &field!(bandwidth_kbps),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.bandwidth_kbps, 50..=aurora_net::throttle::DEFAULT_KBPS as u32)
                        .custom_parser(aurora_i18n::parse_number)
                        .step_by(50.0)
                        .suffix(tr!("bandwidth-kbps-suffix"))
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        // Legacy Viewer › Move & View (FSDisableNeighbourRegionConnections)
        cx.toggle(
            tr!("ui-options-no-neighbour-regions"),
            tr!("ui-options-no-neighbour-regions-help"),
            &field!(no_neighbour_regions),
        );
    });
    cx.group(tr!("ui-options-disk-cache"), |cx| {
        cx.row(
            tr!("ui-options-maximum-size"),
            tr!("ui-options-textures-meshes-animations-shared-across-regions-objects-per-region"),
            &field!(cache_size_mb),
            |ui, s| {
                ui.add(
                    egui::Slider::new(&mut s.cache_size_mb, 512..=65536)
                        .custom_parser(aurora_i18n::parse_number)
                        .logarithmic(true)
                        .suffix(tr!("memory-mb-suffix"))
                        .trailing_fill(true),
                )
                .changed()
            },
        );
        cx.row_with(
            tr!("ui-options-usage"),
            tr!("ui-options-least-recently-used-files-are-removed-when-the-limit"),
            &Nothing,
            |ui, s, st, _| {
                let used = st.cache_usage;
                let limit = s.cache_size_mb as u64 * 1024 * 1024;
                let frac = (used as f32 / limit.max(1) as f32).clamp(0.0, 1.0);
                let text = format!("{} / {}", format_bytes(used), format_bytes(limit));
                widgets::progress(ui, &p, "cache_usage", frac, SLIDER_W + 60.0, &text, None);
                false
            },
        );
        cx.below().row_with(
            tr!("ui-options-cache-folder"),
            tr!("ui-options-cache-folder-help"),
            &fields!(cache_location),
            |ui, s, _, r| {
                let mut changed = false;
                ui.vertical(|ui| {
                    let next = crate::cache_location::root_for(&s.cache_location);
                    path_label(ui, &p, &crate::settings::cache_dir(), ui.available_width());
                    ui.horizontal(|ui| {
                        let size = Vec2::new(0.0, 24.0);
                        if action_button(ui, &p, "folder-filled", tr!("ui-options-change-folder"), ActionKind::Flat, size).clicked() {
                            r.folder = Some((Folder::Cache, FolderChoice::Pick));
                        }
                        if !s.cache_location.is_empty()
                            && action_button(ui, &p, "restore", tr!("ui-options-default-folder"), ActionKind::Flat, size).clicked()
                        {
                            s.cache_location.clear();
                            changed = true;
                        }
                    });
                    if next.as_deref() != crate::cache_location::chosen() {
                        let shown = next.unwrap_or_else(crate::settings::default_cache_root);
                        ui.label(
                            RichText::new(tr!("ui-options-cache-folder-next-start", path = shown.display().to_string()))
                                .size(11.5)
                                .color(p.muted),
                        );
                    }
                });
                changed
            },
        );
        cx.row_with(
            tr!("ui-options-clear-cache"),
            tr!("ui-options-everything-will-be-downloaded-again-as-needed"),
            &Nothing,
            |ui, _, st, r| {
                let label = if st.cache_clearing {
                    tr!("ui-options-deleting")
                } else {
                    tr!("ui-options-clear-cache")
                };
                if ui
                    .add_enabled_ui(!st.cache_clearing, |ui| {
                        action_button(ui, &p, "trash-filled", label, ActionKind::FlatDanger, Vec2::new(0.0, 26.0)).clicked()
                    })
                    .inner
                {
                    r.clear_cache = true;
                }
                false
            },
        );
    });
    cx.group(tr!("ui-options-logs-and-tools"), |cx| {
        cx.row(
            tr!("ui-options-session-log"),
            tr!("ui-options-aurora-log-previous-session-aurora-previous-log"),
            &Nothing,
            |ui, _| {
                if action_button(
                    ui,
                    &p,
                    "folder-open-filled",
                    tr!("ui-options-open-folder"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
                {
                    crate::logging::open_logs_dir();
                }
                false
            },
        );
        // the former Debug page of the window
        cx.row_with(
            tr!("ui-options-debug-tools"),
            tr!("ui-options-debug-tools-help"),
            &Nothing,
            |ui, _, _, r| {
                if action_button(
                    ui,
                    &p,
                    "code-circle-filled",
                    tr!("ui-options-open-dev-toolbox"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
                {
                    r.open_dev_toolbox = true;
                }
                false
            },
        );
    });
}
