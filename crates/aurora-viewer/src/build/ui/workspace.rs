//! Construction presentation and reproducible offline capture geometry.
use super::INSPECTOR_ID;
use crate::ui::widgets;
use egui::{Vec2, vec2};
pub(super) const DEFAULT_SIZE: Vec2 = vec2(350.0, 820.0);
pub(super) const MIN_SIZE: Vec2 = vec2(344.0, 420.0);

/// A reproducible initial size for captures; production uses the floater's
/// ordinary saved geometry; the style override is limited to this demo run.
pub(super) fn prepare(ctx: &egui::Context, demo: bool, s: &mut crate::build::BuildSettings) {
    let id = egui::Id::unique((INSPECTOR_ID, "demo-size"));
    if !demo || ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, true));
    if let Ok(style) = std::env::var("AURORA_DEMO_BUILD_STYLE") {
        match style.as_str() {
            "legacy" => s.legacy_window = true,
            "aurora" => s.legacy_window = false,
            _ => {}
        }
    }
    if let Ok(value) = std::env::var("AURORA_DEMO_BUILD_SIZE")
        && let Some((width, height)) = value.split_once(',')
        && let (Ok(width), Ok(height)) = (width.trim().parse::<f32>(), height.trim().parse::<f32>())
        && width.is_finite()
        && height.is_finite()
        && width > 0.0
        && height > 0.0
    {
        let work = crate::ui::layout::work_area(ctx);
        widgets::place_floater(
            ctx,
            INSPECTOR_ID,
            work.min + vec2(12.0, 12.0),
            vec2(width, height)
                .max(if s.legacy_window { super::legacy::MIN_SIZE } else { MIN_SIZE })
                .min(work.size()),
        );
    }
    if let Ok(tab) = std::env::var("AURORA_DEMO_BUILD_TAB") {
        for extra in tab.split('+').skip(1) {
            let column = match extra.trim() {
                "notice" => {
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("build-demo-notice"), false));
                    continue;
                }
                "drag-option" | "drag-actions" => {
                    let item = if extra.trim() == "drag-actions" {
                        crate::build::layout::Item::Actions
                    } else {
                        crate::build::layout::Item::RootAxis
                    };
                    ctx.data_mut(|data| data.insert_temp(egui::Id::unique("build-demo-layout-drag"), item));
                    continue;
                }
                "default-layout" => {
                    s.layout = Default::default();
                    continue;
                }
                "expanded" => {
                    s.transform_expanded = true;
                    continue;
                }
                "snap-left" => crate::build::layout::FooterColumn::Left,
                "snap-right" => crate::build::layout::FooterColumn::Right,
                _ => continue,
            };
            s.layout.place_in_footer(
                crate::build::layout::Item::Snap,
                column,
                Some(crate::build::layout::Item::Actions),
                false,
            );
        }
    }
}
