//! Single compact construction presentation, with ordinary saved geometry.
use super::INSPECTOR_ID;
use crate::ui::widgets;
use egui::{Vec2, vec2};
pub(super) const DEFAULT_SIZE: Vec2 = vec2(350.0, 820.0);
pub(super) const MIN_SIZE: Vec2 = vec2(344.0, 420.0);

/// A reproducible initial size for captures; production uses the floater's
/// ordinary saved geometry. There is no saved presentation preference.
pub(super) fn prepare(ctx: &egui::Context, demo: bool) {
    let id = egui::Id::unique((INSPECTOR_ID, "demo-size"));
    if !demo || ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(id, true));
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
            vec2(width, height).max(MIN_SIZE).min(work.size()),
        );
    }
}
