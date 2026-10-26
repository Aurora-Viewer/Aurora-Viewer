//! Inventory dragged towards a resident: what a surface that gives it says
//! before the drop. The content fades behind "Drop here to give to <name>"
//! from the start of the drag, wherever the pointer is (#627, #991); the
//! Legacy Viewer only changes the cursor over the surface
//! (LLToolDragAndDrop::handleGiveDragAndDrop). Shared by the private
//! conversations and the profiles.

use crate::theme::Palette;

/// How much the content fades behind the hint, at most.
pub(crate) const DIM: f32 = 0.85;
/// The time the content takes to fade, and to come back.
const FADE: f32 = 0.12;
/// The size of the hint's gift.
const ICON: f32 = 28.0;

/// How far the content of the surface `id` has faded (0 to 1): towards 1
/// while inventory is dragged and the surface gives (`gives`), back to 0
/// once the drag is over.
pub(crate) fn fade(ctx: &egui::Context, id: egui::Id, gives: bool) -> f32 {
    let carried = super::inventory::drag::carried_count(ctx).filter(|_| gives);
    if let Some(n) = carried {
        // kept for the fade back, once the drag is over
        ctx.data_mut(|d| d.insert_temp(id.with("count"), n));
    }
    ctx.animate_bool_with_time(id.with("fade"), carried.is_some(), FADE)
}

/// The hint centred in `area`: to whom a drop gives, and how many entries
/// when several. `over`: the pointer is on the surface, the hint takes the
/// ink.
pub(crate) fn hint(painter: &egui::Painter, p: &Palette, id: egui::Id, area: egui::Rect, over: bool, name: &str, fade: f32) {
    let ctx = painter.ctx();
    let count = ctx.data(|d| d.get_temp::<usize>(id.with("count"))).unwrap_or(1);
    let ink = if over { p.ink } else { p.muted };
    let mut painter = painter.with_clip_rect(area.intersect(painter.clip_rect()));
    painter.multiply_opacity(fade);
    let centred = |text: String, size: f32, color: egui::Color32| {
        let mut job = egui::text::LayoutJob::simple(text, egui::FontId::proportional(size), color, (area.width() - 32.0).max(60.0));
        // centred on x = 0
        job.halign = egui::Align::Center;
        painter.layout_job(job)
    };
    let title = centred(aurora_i18n::tr!("ui-chat-drop-to-give", name = name), 13.5, ink);
    let number = aurora_i18n::integer(count as i64);
    let sub = (count > 1).then(|| {
        centred(
            aurora_i18n::tr!("inventory-items-count", count = count, number = number),
            12.0,
            p.muted,
        )
    });
    let height = ICON + 8.0 + title.size().y + sub.as_ref().map_or(0.0, |s| 3.0 + s.size().y);
    let x = area.center().x;
    let mut y = area.center().y - height / 2.0;
    if let Some(icon) = crate::ui::icons::global("gift-filled") {
        let rect = egui::Rect::from_min_size(egui::pos2(x - ICON / 2.0, y), egui::Vec2::splat(ICON));
        painter.image(
            icon.id(),
            rect,
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            ink,
        );
    }
    y += ICON + 8.0;
    let title_h = title.size().y;
    painter.galley(egui::pos2(x, y), title, ink);
    if let Some(sub) = sub {
        painter.galley(egui::pos2(x, y + title_h + 3.0), sub, p.muted);
    }
}
