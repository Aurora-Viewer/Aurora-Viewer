//! A paint-only cursor image. The payload alone owns the drop, so this follows
//! it across windows without becoming a hover, scroll, or world-picking target.
use super::{Glyph, Inventory, Palette, RowStyle, Uuid, input::Moving, pins::PinDrag};
use crate::ui::widgets;
use egui::{Context, FontId, Id, Pos2, Rect, TextureHandle, Vec2, pos2, vec2};
use std::sync::Arc;

enum Source {
    Rows(Moving),
    Pin(Arc<PinDrag>),
}

impl Source {
    fn current(ctx: &Context) -> Option<Self> {
        Moving::carried(ctx)
            .map(Self::Rows)
            .or_else(|| egui::DragAndDrop::payload::<PinDrag>(ctx).map(Self::Pin))
    }

    fn matches(&self, ctx: &Context) -> bool {
        match self {
            Self::Rows(old) => Moving::carried(ctx).is_some_and(|now| old.same_drag(&now)),
            Self::Pin(old) => egui::DragAndDrop::payload::<PinDrag>(ctx).is_some_and(|now| Arc::ptr_eq(old, &now)),
        }
    }
}

struct Preview {
    root: Uuid,
    source: Source,
    name: String,
    count: usize,
    glyph: Glyph,
    icon: Option<TextureHandle>,
    thumbnail: Option<TextureHandle>,
}

fn key() -> Id {
    Id::unique("explorer-item-drag-preview")
}

pub(super) fn start(ctx: &Context, inv: &Inventory, art: &RowStyle, name: &str, glyph: Glyph, thumbnail: Option<TextureHandle>) {
    let Some(source) = Source::current(ctx) else { return };
    let count = match &source {
        Source::Rows(moving) => {
            let (ids, folders) = moving.entries();
            ids.len() + folders.len()
        }
        Source::Pin(_) => 1,
    };
    let preview = Preview {
        root: inv.root,
        source,
        // Only one line will ever be visible. Bound the layout work on long asset names.
        name: name.chars().take(160).map(|c| if c.is_control() { ' ' } else { c }).collect(),
        count,
        glyph,
        icon: glyph.texture(art.icons).cloned(),
        thumbnail,
    };
    ctx.data_mut(|d| d.insert_temp(key(), Arc::new(preview)));
}

pub(in crate::ui::inventory) fn cancel(ctx: &Context) {
    let preview = ctx.data_mut(|data| {
        let preview = data.get_temp::<Arc<Preview>>(key());
        data.remove::<Arc<Preview>>(key());
        preview
    });
    if preview.is_some_and(|p| p.source.matches(ctx)) {
        egui::DragAndDrop::clear_payload(ctx);
        ctx.stop_dragging();
    }
}

pub(in crate::ui::inventory) fn show(ctx: &Context, p: &Palette, inv: &Inventory) -> bool {
    let Some(preview) = ctx.data(|d| d.get_temp::<Arc<Preview>>(key())) else {
        return false;
    };
    let (held, pointer) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
    if !held || inv.root != preview.root || !preview.source.matches(ctx) {
        ctx.data_mut(|d| d.remove::<Arc<Preview>>(key()));
        return false;
    }
    if let Some(pos) = pointer.filter(|pos| ctx.content_rect().contains(*pos)) {
        paint(ctx, p, pos, &preview, inv.can_preview(0));
    }
    true
}

fn position(screen: Rect, pointer: Pos2, size: Vec2, hint: Option<Rect>) -> Rect {
    let gap = 10.0;
    let place = |x: f32, y: f32| {
        let min = pos2(x, y).min(screen.max - size).max(screen.min);
        Rect::from_min_size(min, size)
    };
    let right = pointer.x + 15.0;
    let left = pointer.x - size.x - 8.0;
    let above = pointer.y - size.y - gap;
    let below = pointer.y + 22.0;
    let preferred = if right + size.x <= screen.right() { right } else { left };
    let opposite = if preferred == right { left } else { right };
    let above_first = above >= screen.top();
    let ys = if above_first { [above, below] } else { [below, above] };
    let good = |r: Rect| !r.contains(pointer) && hint.is_none_or(|h| !r.intersects(h.expand(3.0)));
    for y in ys {
        for x in [preferred, opposite] {
            let r = place(x, y);
            if good(r) {
                return r;
            }
        }
    }
    if let Some(h) = hint {
        for (x, y) in [
            (preferred, h.bottom() + gap),
            (preferred, h.top() - size.y - gap),
            (h.left() - size.x - gap, below),
            (h.right() + gap, below),
        ] {
            let r = place(x, y);
            if good(r) {
                return r;
            }
        }
    }
    place(preferred, ys[0])
}

fn paint(ctx: &Context, p: &Palette, pointer: Pos2, preview: &Preview, thumbnail_allowed: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, key()));
    let screen = ctx.content_rect();
    let count = (preview.count > 1)
        .then(|| painter.layout_no_wrap(aurora_i18n::integer(preview.count as i64), FontId::proportional(12.0), p.on_accent));
    let count_width = count.as_ref().map_or(0.0, |g| (g.size().x + 10.0).max(20.0) + 6.0);
    let side = 25.0;
    let fixed_width = side + 7.0 + 14.0 + count_width;
    let name_width = (screen.width() - fixed_width).clamp(1.0, 184.0);
    let mut job = egui::text::LayoutJob::simple(preview.name.clone(), FontId::proportional(13.0), p.ink, name_width);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let name = painter.layout_job(job);
    let size = vec2(
        (fixed_width + name.size().x).min(screen.width()),
        (side + 10.0).max(name.size().y + 8.0),
    );
    let hint = super::super::drag::hint_rect(ctx);
    let rect = position(screen, pointer, size, hint);
    let painter = painter.with_clip_rect(rect.intersect(screen));
    painter.rect(
        rect,
        ctx.global_style().visuals.window_corner_radius,
        p.panel,
        egui::Stroke::new(1.0, p.muted_dim),
        egui::StrokeKind::Inside,
    );
    let center = pos2(rect.left() + 7.0 + side * 0.5, rect.center().y);
    let icon_rect = Rect::from_center_size(center, Vec2::splat(side));
    if let Some(image) = preview.thumbnail.as_ref().filter(|_| thumbnail_allowed) {
        let native = image.size_vec2().max(Vec2::splat(1.0));
        let size = native * (side / native.x.max(native.y));
        painter.image(
            image.id(),
            Rect::from_center_size(center, size),
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            widgets::PICTURE_TINT,
        );
    } else {
        preview.glyph.paint(&painter, preview.icon.as_ref(), p.ink, icon_rect);
    }
    crate::ui::widgets::paint_row_text(&painter, name, icon_rect.right() + 7.0, center.y, p.ink);
    if let Some(count) = count {
        let badge = Rect::from_center_size(
            pos2(rect.right() - 7.0 - (count_width - 6.0) * 0.5, center.y),
            vec2(count_width - 6.0, 20.0),
        );
        painter.rect_filled(badge, ctx.global_style().visuals.widgets.inactive.corner_radius, p.violet);
        let x = badge.center().x - count.size().x * 0.5;
        crate::ui::widgets::paint_row_text(&painter, count, x, center.y, p.on_accent);
    }
}
